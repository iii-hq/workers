use crate::context::E2eContext;

use super::names::ScenarioNames;
use super::queries;
use crate::scenarios::CleanupFuture;

pub(super) fn run<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let names = ScenarioNames::new(run_id);
        for table in queries::existing_tables(context, &names).await? {
            if !table.starts_with(&format!("{}_", names.table_prefix))
                || !table
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                continue;
            }
            queries::drop_table(context, &table).await?;
        }
        Ok(())
    })
}
