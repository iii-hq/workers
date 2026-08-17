use super::EXPECTED_WRITERS;

pub(super) struct ScenarioNames {
    pub(super) run_label: String,
    pub(super) table_prefix: String,
    pub(super) orders: String,
    pub(super) writers: String,
    pub(super) totals: String,
    pub(super) report: String,
    pub(super) writer_sessions: [String; EXPECTED_WRITERS],
    pub(super) finalizer_session: String,
}

impl ScenarioNames {
    pub(super) fn new(run_id: &str) -> Self {
        let mut suffix: String = run_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(4)
            .collect();
        while suffix.len() < 4 {
            suffix.push('0');
        }
        let run_label = format!("rctest-{suffix}");
        let table_prefix = format!("rctest_{suffix}");
        Self {
            orders: format!("{table_prefix}_orders"),
            writers: format!("{table_prefix}_writers"),
            totals: format!("{table_prefix}_totals"),
            report: format!("{table_prefix}_report"),
            writer_sessions: [
                format!("{run_label}-writer-1"),
                format!("{run_label}-writer-2"),
                format!("{run_label}-writer-3"),
            ],
            finalizer_session: format!("{run_label}-finalizer"),
            run_label,
            table_prefix,
        }
    }
}
