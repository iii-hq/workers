//! The `learned` rule's size cap. Extraction and consolidation keep
//! appending to it; past `learned_max_chars` its oldest lines move back to
//! memories, so it stays bounded.

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::extract::LEARNED_RULE;

/// Tag on a memory a `learned` line was moved into.
pub const DEMOTED_TAG: &str = "demoted-rule";

/// One learned write, merge or demotion at a time in this process (one
/// worker replica, like the extraction session locks).
pub(crate) static LEARNED_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Move the oldest lines of `bank_name`'s `learned` rule into memories tagged
/// [`DEMOTED_TAG`] until the rule, measured as injected, fits
/// `learned_max_chars`. Each line is saved before the rule is rewritten and
/// only saved lines leave it; one that cannot be saved stays and the next
/// oldest goes instead; the newest line always stays. Returns how many moved.
pub(crate) async fn demote_over_cap(deps: &Deps, bank_name: &str) -> Result<usize, MemoryError> {
    let cfg = deps.config().await;
    let cap = cfg.learned_max_chars;
    if cap == 0 {
        return Ok(0);
    }
    let _writes = LEARNED_WRITES.lock().await;
    let store = deps.store().await;
    let current = crate::extract::learned_of(&store, bank_name).await;
    let mut lines: Vec<&str> = current.lines().collect();
    let injected = |lines: &[&str]| {
        crate::hooks::rule_body(&crate::types::Rule {
            name: LEARNED_RULE.into(),
            content: lines.join("\n"),
            updated_at: 0,
        })
        .len()
    };
    // A default-bank line was read under every repository bank: so is its memory.
    let entities = if bank_name == cfg.default_bank {
        vec![crate::scope::LAYER_ENTITY.to_string()]
    } else {
        Vec::new()
    };
    let (mut from, mut moved) = (0, 0);
    // shortcut: a hand-written bullet's indented continuation lines stay behind when it moves (the injection cut has the same limit), move them with it if people write multi-line rules.
    while injected(&lines) > cap {
        let Some(i) = (from..lines.len()).find(|&i| lines[i].starts_with("- ")) else {
            break;
        };
        if !lines[i + 1..].iter().any(|l| l.starts_with("- ")) {
            break;
        }
        let req = crate::functions::items::SaveRequest {
            bank: Some(bank_name.into()),
            text: lines[i]["- ".len()..].to_string(),
            entities: entities.clone(),
            tags: vec![DEMOTED_TAG.into()],
            pinned: false,
            session_id: None,
        };
        match crate::functions::items::save_in(deps, req, true).await {
            Ok(_) => {
                lines.remove(i);
                moved += 1;
            }
            // Trashed since the write: it stays gone, and has no rule to trim.
            Err(MemoryError::BankNotFound(_)) => return Ok(0),
            // A line whose memory was superseded too: a line has no date, so
            // it may be the user's newest statement, written after the correction.
            Err(e) => {
                tracing::warn!(bank = %bank_name, error = %e, "a learned line could not be demoted; it stays");
                from = i + 1;
            }
        }
    }
    if moved == 0 {
        return Ok(0);
    }
    let trimmed = lines.join("\n") + "\n";
    let bank = store.bank(bank_name).await?;
    // shortcut: no retry when the rule changed under the rewrite; only an edit outside the worker (by hand, another replica) can, as every write through it waits for this lock, and the lines saved above stay memories too; re-merge onto such edits if they happen.
    let written = crate::store::run_blocking(|| {
        bank.extraction_rule_expected(LEARNED_RULE, &current, &trimmed)
    })
    .await?;
    if written.is_none() {
        tracing::warn!(bank = %bank_name, "learned rule changed during demotion; left as it is");
        return Ok(0);
    }
    deps.emitter.bank("rules-changed", bank_name).await;
    tracing::info!(bank = %bank_name, moved, "learned rule over its cap; oldest lines demoted to memories");
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::types::fingerprint;

    fn offline_deps(dir: &std::path::Path, learned_max_chars: usize) -> Deps {
        let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
        let cfg = WorkerConfig {
            learned_max_chars,
            embeddings_enabled: false,
            ..WorkerConfig::default()
        };
        Deps {
            hook_binding: Arc::default(),
            emitter: Arc::new(crate::events::Emitter::new(iii.clone())),
            iii,
            store: Arc::new(tokio::sync::RwLock::new(Arc::new(
                crate::store::Store::open(dir.to_path_buf()).unwrap(),
            ))),
            config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        }
    }

    fn numbered(count: usize) -> Vec<String> {
        (1..=count)
            .map(|n| format!("Learned line number {n:02}, padded to some length."))
            .collect()
    }

    async fn seed(deps: &Deps, bank: &str, lines: &[String]) {
        let store = deps.store().await;
        let (b, _) = store.ensure_bank(bank, None).await.unwrap();
        let body: String = lines.iter().map(|l| format!("- {l}\n")).collect();
        b.set_rule(
            LEARNED_RULE,
            &format!("{}{body}", crate::extract::LEARNED_HEADER),
        )
        .unwrap();
    }

    async fn learned(deps: &Deps, bank: &str) -> String {
        crate::extract::learned_of(&*deps.store().await, bank).await
    }

    fn kept(rule: &str) -> Vec<String> {
        rule.lines()
            .filter_map(|l| l.strip_prefix("- "))
            .map(str::to_string)
            .collect()
    }

    fn injected(rule: &str) -> usize {
        crate::hooks::rule_body(&crate::types::Rule {
            name: LEARNED_RULE.into(),
            content: rule.into(),
            updated_at: 0,
        })
        .len()
    }

    #[tokio::test]
    async fn an_over_cap_learned_rule_demotes_its_oldest_lines_none_lost() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        let lines = numbered(12);
        seed(&deps, "main", &lines).await;
        // Already a memory, but not one every repository reads.
        let held = crate::functions::items::SaveRequest {
            bank: Some("main".into()),
            text: lines[0].clone(),
            entities: vec!["amber".into()],
            tags: Vec::new(),
            pinned: false,
            session_id: None,
        };
        crate::functions::items::save(&deps, held).await.unwrap();

        let moved = demote_over_cap(&deps, "main").await.unwrap();
        let rule = learned(&deps, "main").await;
        assert!(injected(&rule) <= 300, "{rule}");
        let kept = kept(&rule);
        assert_eq!(moved + kept.len(), 12);
        assert!(moved > 0);
        assert_eq!(kept, lines[moved..], "the newest stay, in order");
        let bank = deps.store().await.bank("main").await.unwrap();
        for line in &lines[..moved] {
            let memory = bank.get(&fingerprint(line)).await.expect(line);
            assert!(memory.is_live(), "{line}");
            assert!(memory.tags.iter().any(|t| t == DEMOTED_TAG), "{line}");
            assert!(memory.entities.iter().any(|e| e == "user"), "{line}");
        }
        let held = bank.get(&fingerprint(&lines[0])).await.unwrap();
        assert_eq!(held.entities, ["amber", "user"], "read where the line was");
        assert_eq!(held.corroboration, 1);
        assert!(rule.starts_with(crate::extract::LEARNED_HEADER), "{rule}");

        assert_eq!(demote_over_cap(&deps, "main").await.unwrap(), 0);
        assert_eq!(learned(&deps, "main").await, rule);
    }

    #[tokio::test]
    async fn a_demotion_under_turn_baggage_still_writes_its_lines() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        let lines = numbered(12);
        seed(&deps, "main", &lines).await;
        // The queue hands the extraction job the turn's baggage; this
        // process never saw that session or turn.
        let moved = crate::scope::in_turn("s-job", "t-job", demote_over_cap(&deps, "main"))
            .await
            .unwrap();
        assert!(moved > 0);
        let bank = deps.store().await.bank("main").await.unwrap();
        for line in &lines[..moved] {
            let memory = bank.get(&fingerprint(line)).await.expect(line);
            assert!(memory.is_live(), "{line}");
            assert!(memory.tags.iter().any(|t| t == DEMOTED_TAG), "{line}");
            assert!(memory.entities.iter().any(|e| e == "user"), "{line}");
            assert_eq!(memory.source, None, "the worker's own write: {line}");
            assert!(!memory.is_agent_note(), "{line}");
        }
    }

    #[tokio::test]
    async fn a_line_added_back_after_its_memory_was_superseded_stays() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        let lines = numbered(12);
        let saved = |text: &str| crate::functions::items::SaveRequest {
            bank: Some("main".into()),
            text: text.into(),
            entities: vec!["amber".into()],
            tags: Vec::new(),
            pinned: false,
            session_id: None,
        };
        let save = |text: &str| crate::functions::items::save(&deps, saved(text));
        let old = save(&lines[0]).await.unwrap().memory;
        let correction = save("Learned line number 01 was wrong.")
            .await
            .unwrap()
            .memory;
        let req = crate::functions::items::SupersedeRequest {
            bank: Some("main".into()),
            id: old.id.clone(),
            superseded_by: correction.id.clone(),
            expected_memory: None,
            expected_superseded_by: None,
            reinforce: false,
            reason: None,
        };
        crate::functions::items::supersede(&deps, req)
            .await
            .unwrap();
        // The user said it again: the line is newer than the correction.
        seed(&deps, "main", &lines).await;

        let moved = demote_over_cap(&deps, "main").await.unwrap();
        assert!(moved > 0);
        let rule = learned(&deps, "main").await;
        assert!(injected(&rule) <= 300, "{rule}");
        let kept = kept(&rule);
        assert_eq!(kept[0], lines[0], "{rule}");
        assert_eq!(kept[1..], lines[1 + moved..]);
        let bank = deps.store().await.bank("main").await.unwrap();
        assert!(!bank.get(&old.id).await.unwrap().is_live());
        assert!(bank.get(&correction.id).await.unwrap().is_live());
        assert!(bank.get(&fingerprint(&lines[1])).await.unwrap().is_live());
    }

    #[tokio::test]
    async fn a_line_that_cannot_be_saved_stays_and_the_next_oldest_goes() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        // Under the 3-char floor of a memory.
        let mut lines = vec!["ab".to_string()];
        lines.extend(numbered(12));
        seed(&deps, "main", &lines).await;

        let moved = demote_over_cap(&deps, "main").await.unwrap();
        let rule = learned(&deps, "main").await;
        assert!(injected(&rule) <= 300, "{rule}");
        let kept = kept(&rule);
        assert_eq!(kept[0], "ab", "{rule}");
        assert_eq!(kept[1..], lines[1 + moved..]);
    }

    #[tokio::test]
    async fn a_zero_cap_demotes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 0);
        seed(&deps, "main", &numbered(12)).await;
        let before = learned(&deps, "main").await;
        assert_eq!(demote_over_cap(&deps, "main").await.unwrap(), 0);
        assert_eq!(learned(&deps, "main").await, before);
    }

    #[tokio::test]
    async fn setting_an_over_cap_learned_rule_demotes_through_rule_set() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        let lines = numbered(12);
        let body: String = lines.iter().map(|l| format!("- {l}\n")).collect();
        crate::functions::rules::set(
            &deps,
            crate::functions::rules::RuleSetRequest {
                bank: Some("repo".into()),
                name: LEARNED_RULE.into(),
                content: body,
                expected: Some(String::new()),
            },
        )
        .await
        .unwrap();
        let rule = learned(&deps, "repo").await;
        assert!(injected(&rule) <= 300, "{rule}");
        let moved = 12 - kept(&rule).len();
        assert!(moved > 0, "{rule}");
        let bank = deps.store().await.bank("repo").await.unwrap();
        let oldest = bank.get(&fingerprint(&lines[0])).await.unwrap();
        assert!(oldest.tags.iter().any(|t| t == DEMOTED_TAG));
        assert!(oldest.entities.is_empty(), "a repository bank's own line");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_guarded_edit_waits_for_a_running_demotion() {
        let dir = tempfile::tempdir().unwrap();
        let deps = Arc::new(offline_deps(dir.path(), 300));
        let lines = numbered(60);
        seed(&deps, "main", &lines).await;
        let before = learned(&deps, "main").await;
        let bank = deps.store().await.bank("main").await.unwrap();

        let demotion = tokio::spawn({
            let deps = deps.clone();
            async move { demote_over_cap(&deps, "main").await }
        });
        // The demotion has read the rule and is saving its lines.
        while bank.get(&fingerprint(&lines[0])).await.is_none() && !demotion.is_finished() {
            tokio::task::yield_now().await;
        }
        // The editor deletes the oldest line from the rule it last read.
        let edit = crate::functions::rules::set(
            &deps,
            crate::functions::rules::RuleSetRequest {
                bank: Some("main".into()),
                name: LEARNED_RULE.into(),
                content: before.replacen(&format!("- {}\n", lines[0]), "", 1),
                expected: Some(before.clone()),
            },
        )
        .await;
        let moved = demotion.await.unwrap().unwrap();

        let err = edit.expect_err("the edit lands after the demotion, on a changed rule");
        assert!(err.to_string().contains("rule_changed"), "{err}");
        let rule = learned(&deps, "main").await;
        assert!(injected(&rule) <= 300, "{rule}");
        assert_eq!(kept(&rule), lines[moved..]);
        for line in &lines[..moved] {
            let memory = bank.get(&fingerprint(line)).await.expect(line);
            assert_eq!(memory.corroboration, 0, "saved once: {line}");
        }
    }

    #[tokio::test]
    async fn a_demotion_never_brings_back_a_trashed_bank() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path(), 300);
        seed(&deps, "repo", &numbered(12)).await;
        let store = deps.store().await;
        store.trash_bank("repo", "main").await.unwrap();
        let req = crate::functions::items::SaveRequest {
            bank: Some("repo".into()),
            text: "Learned line number 01, padded to some length.".into(),
            entities: Vec::new(),
            tags: vec![DEMOTED_TAG.into()],
            pinned: false,
            session_id: None,
        };
        let err = crate::functions::items::save_in(&deps, req, true)
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::BankNotFound(_)), "{err}");
        assert!(store.bank("repo").await.is_err(), "still trashed");
    }
}
