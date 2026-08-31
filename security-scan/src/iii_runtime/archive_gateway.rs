use super::*;

impl IiiRuntime {
    pub fn set_archive(&self, archive: Option<ArchiveConfigV1>) {
        *self
            .archive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = archive;
    }

    fn archive_config(&self) -> Option<ArchiveConfigV1> {
        self.archive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) async fn archive_run(&self, run: &RunRecordV1) {
        let Some(archive) = self.archive_config() else {
            return;
        };
        let key = match archive::object_key(archive.prefix.as_deref(), &run.run_id) {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(run_id = %run.run_id, %error, "security scan archive skipped");
                return;
            }
        };
        if let Err(error) = self.remember_archived_run(&run.run_id).await {
            tracing::warn!(
                run_id = %run.run_id,
                bucket = %archive.bucket,
                %error,
                "security scan archive index update failed"
            );
            return;
        }
        let body_base64 = match archive::encode_run(run) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(run_id = %run.run_id, %error, "security scan archive skipped");
                return;
            }
        };
        if let Err(error) = self
            .put_archived_object(&archive.bucket, &key, &body_base64)
            .await
        {
            tracing::warn!(
                run_id = %run.run_id,
                bucket = %archive.bucket,
                %error,
                "security scan archive write deferred for repair"
            );
        }
    }

    pub async fn repair_archived_runs(&self) -> Result<usize, SecurityScanError> {
        let Some(archive) = self.archive_config() else {
            return Ok(0);
        };
        let index = self
            .call_private(STATE_LIST_ID, json!({ "scope": ARCHIVE_INDEX_SCOPE }))
            .await?;
        let records: Vec<archive::ArchiveIndexRecordV1> =
            parse_state_list(&index, "archive index")?;
        let mut repaired = 0;
        for record in records {
            let Some(run) = self.get_run(&record.run_id).await? else {
                continue;
            };
            let key = archive::object_key(archive.prefix.as_deref(), &record.run_id)?;
            let body = archive::encode_run(&run)?;
            if self
                .get_archived_object(&archive.bucket, &key)
                .await?
                .as_deref()
                == Some(body.as_str())
            {
                continue;
            }
            self.put_archived_object(&archive.bucket, &key, &body)
                .await?;
            repaired += 1;
        }
        Ok(repaired)
    }

    pub async fn import_archived_runs(&self) -> Result<usize, SecurityScanError> {
        let Some(archive) = self.archive_config() else {
            return Ok(0);
        };
        let index = self
            .call_private(STATE_LIST_ID, json!({ "scope": ARCHIVE_INDEX_SCOPE }))
            .await?;
        let mut records: Vec<archive::ArchiveIndexRecordV1> =
            parse_state_list(&index, "archive index")?;
        if records.is_empty() {
            if let Some(body) = self
                .get_archived_object(
                    &archive.bucket,
                    &archive::legacy_manifest_key(archive.prefix.as_deref()),
                )
                .await?
            {
                for run_id in archive::decode_legacy_manifest(&body)?.run_ids {
                    self.remember_archived_run(&run_id).await?;
                    records.push(archive::index_record(&run_id));
                }
            }
        }
        let mut imported = 0;
        for record in records {
            let run_id = record.run_id;
            let key = match archive::object_key(archive.prefix.as_deref(), &run_id) {
                Ok(key) => key,
                Err(error) => {
                    tracing::warn!(run_id = %run_id, %error, "skipped archived security scan");
                    continue;
                }
            };
            let Some(body) = self.get_archived_object(&archive.bucket, &key).await? else {
                tracing::warn!(key = %key, "archived security scan object is missing");
                continue;
            };
            let run = match archive::decode_run(&body) {
                Ok(run) => run,
                Err(error) => {
                    tracing::warn!(key = %key, %error, "skipped archived security scan");
                    continue;
                }
            };
            match self.create_run_if_absent(run).await? {
                CreateRunOutcome::Created => imported += 1,
                CreateRunOutcome::Existing(_) => {}
            }
        }
        Ok(imported)
    }

    async fn remember_archived_run(&self, run_id: &str) -> Result<(), SecurityScanError> {
        let value = serialize(&archive::index_record(run_id), "archive index record")?;
        match self
            .compare_and_set_in_scope(ARCHIVE_INDEX_SCOPE, run_id, None, Some(value.clone()))
            .await?
        {
            CasOutcome::Swapped => Ok(()),
            CasOutcome::Current(current) if current == value => Ok(()),
            CasOutcome::Current(_) => Err(SecurityScanError::Dependency(format!(
                "archive index collision for run {run_id}"
            ))),
        }
    }

    async fn put_archived_object(
        &self,
        bucket: &str,
        key: &str,
        body_base64: &str,
    ) -> Result<(), SecurityScanError> {
        self.call(
            STORAGE_PUT_ID,
            json!({
                "bucket": bucket,
                "key": key,
                "body_base64": body_base64,
                "content_type": "application/json",
            }),
            None,
            Some(RPC_TIMEOUT_MS),
        )
        .await
        .map(|_| ())
    }

    async fn get_archived_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<String>, SecurityScanError> {
        match self
            .call(
                STORAGE_GET_ID,
                json!({
                    "bucket": bucket,
                    "key": key,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await
        {
            Ok(value) => {
                let fetched: StorageGetWire = serde_json::from_value(value)
                    .map_err(|error| dependency_parse(STORAGE_GET_ID, error))?;
                Ok(Some(fetched.body_base64))
            }
            Err(error) if is_object_not_found(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}
