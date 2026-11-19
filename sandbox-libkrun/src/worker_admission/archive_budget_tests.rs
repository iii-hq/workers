// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (R10). Actual catalog operations, runtime sentinels and
//! archive moves; no fabricated retirement, no raised Directory::names bound.
use super::*;
use crate::worker_admission::testing;
use std::path::PathBuf;
fn context() -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    PathBuf,
) {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    std::fs::create_dir(&ns).unwrap();
    std::fs::set_permissions(&ns, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let context = testing::provision_context(&ns).unwrap();
    let root = temp.path().join("data");
    std::fs::create_dir(&root).unwrap();
    // Exact private mode, never the process umask.
    std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    (temp, context, root)
}
/// A closed operation whose `count` prepared runtimes were cancelled before
/// spawn: each leaves one real `rt-*` sentinel that archiving must move.
fn closed_with_runtimes(
    ctx: &crate::worker_admission::AdmissionContext,
    path: &std::path::Path,
    count: usize,
) -> String {
    let lease = ctx.local(path).unwrap();
    let id = lease.0.id.clone();
    for _ in 0..count {
        drop(lease.prepare_runtime(&[]).unwrap());
    }
    drop(lease);
    id
}
fn request(ctx: &crate::worker_admission::AdmissionContext, id: &str) -> RecoveryRequest {
    let tx = ctx.namespace.transaction().unwrap();
    RecoveryRequest {
        operation_id: id.into(),
        expected_record_sha256: digest(tx.operation(id).unwrap()).unwrap(),
        action: RecoveryAction::ArchiveCompleted,
    }
}
fn catalog_bytes(ctx: &crate::worker_admission::AdmissionContext) -> Vec<u8> {
    std::fs::read(ctx.namespace.directory.path.join("catalog.json")).unwrap()
}
/// The budget refusal is the metadata limit, never an incidental `Busy`.
fn assert_budget_refusal(result: io::Result<RecoveryReport>) {
    let error = result.err().expect("over-budget archive must be refused");
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), limit().to_string());
}
#[test]
fn r10_aggregate_name_union_exact_fit_one_over_and_idempotent_names() {
    let mut union = (0..2045)
        .map(|i| (format!("old-{i}"), 1u64))
        .collect::<std::collections::BTreeMap<_, _>>();
    let new = vec![
        ("new.stage".to_string(), 20),
        ("new.json".to_string(), 20),
        ("op-new".to_string(), 10),
    ];
    merge_reservation(&mut union, new.clone()).unwrap();
    assert_eq!(union.len(), 2048);
    // Idempotent retry names count once.
    merge_reservation(&mut union, new).unwrap();
    assert_eq!(union.len(), 2048);
    assert!(merge_reservation(&mut union, [("one-over".into(), 1)]).is_err());
}
#[test]
fn r10_many_small_completed_archives_and_runtime_sentinels_reach_real_name_boundary() {
    let (_t, ctx, data) = context();
    // Seven terminal operations with 256 runtimes each archive 7 * 258 = 1806
    // names; one with 237 runtimes adds 239, so `archive/` holds 2045 names and
    // an in-flight archive needing exactly 3 (stage, bundle, sentinel) fits.
    for n in 0..7 {
        let id = closed_with_runtimes(&ctx, &data.join(format!("many-{n}")), 256);
        recover_test(&ctx, request(&ctx, &id)).unwrap();
    }
    let id = closed_with_runtimes(&ctx, &data.join("edge"), 237);
    recover_test(&ctx, request(&ctx, &id)).unwrap();
    assert_eq!(ctx.namespace.archive.names().unwrap().len(), 2045);
    // Exact fit: the reservation is durable before the injected post-intent
    // fault, so the in-flight work owns its names across the interruption.
    let first = closed_with_runtimes(&ctx, &data.join("pending"), 0);
    assert!(recover_fault_test(&ctx, request(&ctx, &first), "archive-intent").is_err());
    {
        let tx = ctx.namespace.transaction().unwrap();
        assert_eq!(
            archive_usage(&ctx.namespace, &tx.catalog).unwrap().len(),
            2048
        );
    }
    // Over budget: refused before ANY intent or effect, catalog untouched,
    // ordinary inspection still usable.
    let blocked = closed_with_runtimes(&ctx, &data.join("blocked"), 0);
    let before = catalog_bytes(&ctx);
    let names = || {
        ctx.namespace
            .archive
            .names()
            .unwrap()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    };
    let archived = names();
    assert_budget_refusal(recover_test(&ctx, request(&ctx, &blocked)));
    assert_eq!(catalog_bytes(&ctx), before);
    assert_eq!(names(), archived);
    assert!(
        !ctx.namespace
            .transaction()
            .unwrap()
            .catalog
            .archives
            .iter()
            .any(|a| a.operation == blocked)
    );
    assert!(inspect_test(&ctx).is_ok());
    // Resume reuses the same reservation (no over-reservation) and completes.
    recover_test(&ctx, request(&ctx, &first)).unwrap();
    assert_eq!(ctx.namespace.archive.names().unwrap().len(), 2047);
    assert_budget_refusal(recover_test(&ctx, request(&ctx, &blocked)));
    assert!(inspect_test(&ctx).is_ok());
}
#[test]
fn r10_archive_resume_reuses_exact_reserved_union_at_every_boundary() {
    for at in [
        "archive-intent",
        "archive-stage",
        "archive-promoted",
        "archive-receipt",
        "archive-sentinels",
    ] {
        let (_t, ctx, data) = context();
        let id = closed_with_runtimes(&ctx, &data.join("root"), 3);
        assert!(recover_fault_test(&ctx, request(&ctx, &id), at).is_err());
        let reserved = {
            let tx = ctx.namespace.transaction().unwrap();
            let a = tx
                .catalog
                .archives
                .iter()
                .find(|a| a.operation == id)
                .unwrap();
            // stage + bundle + op sentinel + three runtime sentinels.
            assert_eq!(a.reservation.as_ref().unwrap().names.len(), 6);
            assert!(
                a.reservation
                    .as_ref()
                    .unwrap()
                    .names
                    .contains_key(&format!("{id}.stage"))
            );
            archive_usage(&ctx.namespace, &tx.catalog).unwrap().len()
        };
        // Stage/bundle/sentinels already moved are counted once, not twice.
        assert_eq!(reserved, 6);
        recover_test(&ctx, request(&ctx, &id)).unwrap();
        let tx = ctx.namespace.transaction().unwrap();
        let names = &tx.catalog.archives[0].reservation.as_ref().unwrap().names;
        // Completion releases only the temporary stage name.
        assert_eq!(names.len(), 5);
        assert!(!names.contains_key(&format!("{id}.stage")));
        assert_eq!(ctx.namespace.archive.names().unwrap().len(), 5);
    }
}
#[test]
fn r10_metadata_bytes_reservation_has_no_silent_overflow_or_duplicate_charge() {
    let mut union = std::collections::BTreeMap::new();
    merge_reservation(&mut union, [("bundle".into(), ARCHIVE_BYTE_BUDGET)]).unwrap();
    merge_reservation(&mut union, [("bundle".into(), 1)]).unwrap();
    assert_eq!(union["bundle"], ARCHIVE_BYTE_BUDGET);
    assert!(merge_reservation(&mut union, [("extra".into(), 1)]).is_err());
    let mut overflow = std::collections::BTreeMap::new();
    assert!(
        merge_reservation(
            &mut overflow,
            [("a".to_string(), u64::MAX), ("b".to_string(), u64::MAX)]
        )
        .is_err()
    );
}
#[test]
fn r10_many_small_archives_do_not_charge_duplicate_names_or_wedge_inspection() {
    let (_t, ctx, data) = context();
    for n in 0..300 {
        let id = closed_with_runtimes(&ctx, &data.join(format!("small-{n}")), 0);
        recover_test(&ctx, request(&ctx, &id)).unwrap();
    }
    let tx = ctx.namespace.transaction().unwrap();
    assert_eq!(
        archive_usage(&ctx.namespace, &tx.catalog).unwrap().len(),
        600
    );
    assert_eq!(ctx.namespace.archive.names().unwrap().len(), 600);
    drop(tx);
    assert!(inspect_test(&ctx).is_ok());
}
#[test]
fn r10_reservation_structure_is_validated_without_an_aggregate_wedge() {
    let (_t, ctx, data) = context();
    let id = closed_with_runtimes(&ctx, &data.join("root"), 1);
    assert!(recover_fault_test(&ctx, request(&ctx, &id), "archive-intent").is_err());
    let tx = ctx.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .archives
        .iter()
        .find(|a| a.operation == id)
        .unwrap()
        .clone();
    drop(tx);
    validate_reservation(&record, record.reservation.as_ref().unwrap()).unwrap();
    // A reservation naming anything but its exact record names is corrupt.
    let mut wrong = record.reservation.clone().unwrap();
    wrong.names.insert("foreign".into(), 1);
    assert!(validate_reservation(&record, &wrong).is_err());
    let mut missing = record.reservation.clone().unwrap();
    missing.names.remove(&format!("{id}.stage"));
    assert!(validate_reservation(&record, &missing).is_err());
}
