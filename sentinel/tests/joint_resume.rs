#[path = "support/db.rs"]
mod db;
use sentinel::{Db, Statement, Store};
use serde_json::json;

#[tokio::test]
async fn legacy_running_member_is_backfilled_and_blocks_joint_reservation() {
    let db = db::test_db().await;
    db.execute(sentinel::store::schema::META_TABLE, vec![])
        .await
        .unwrap();
    for (version, statements) in sentinel::store::schema::migrations().into_iter().take(2) {
        let mut steps: Vec<_> = statements
            .into_iter()
            .map(|sql| Statement::new(sql, vec![]))
            .collect();
        steps.push(Statement::new("INSERT INTO sentinel_meta (key,value) VALUES ('schema_version',?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",vec![json!(version.to_string())]));
        db.transaction(&steps).await.unwrap();
    }
    db.execute("INSERT INTO sentinel_groups (id,fingerprint,source,service_name,title,message_sample,status,first_seen_ms,last_seen_ms,updated_ms) VALUES ('legacy','legacy','trace','worker','legacy','legacy','new',1,1,1)",vec![]).await.unwrap();
    db.execute("INSERT INTO sentinel_investigations (id,group_id,occurrence_id,session_id,mode,model,status,created_ms) VALUES ('legacy-inv','legacy','occ','sentinel-inv-legacy','assisted','m','running',1)",vec![]).await.unwrap();
    let store = Store::new(db);
    store.migrate().await.unwrap();
    let members=store.db().query("SELECT investigation_id FROM sentinel_investigation_members WHERE group_id = 'legacy' AND active = 1",vec![]).await.unwrap();
    assert_eq!(
        members.len(),
        1,
        "migration must reserve pre-v3 running investigations"
    );
    assert_eq!(members[0]["investigation_id"], "legacy-inv");
    assert!(store.db().execute("INSERT INTO sentinel_investigation_members (investigation_id,group_id,evidence,active) VALUES ('legacy-inv','legacy','',1)",vec![]).await.is_err());
}
