use std::sync::Arc;

use iii_sdk::IIIClient;
use iii_sdk::errors::Error;

use crate::clients::telegram;
use crate::deps::Deps;
use crate::kv;
use crate::types::PendingApprovalRecord;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::super::register_observed(
        iii,
        deps,
        super::super::ON_PENDING_CREATED_ID,
        "Send an inline approval keyboard when a function call is held.",
        |d, record| async move { handle_internal(&d, record).await },
    );
}

pub async fn handle_internal(
    deps: &Deps,
    record: PendingApprovalRecord,
) -> Result<BindingAck, Error> {
    let Some(chat_id) = kv::chat_id_for_session(deps, &record.session_id).await else {
        return Ok(BindingAck { ok: true });
    };

    let token = short_token(&record.function_call_id);
    kv::store_approval_callback(
        deps,
        &token,
        &crate::types::ApprovalCallbackData {
            session_id: record.session_id.clone(),
            function_call_id: record.function_call_id.clone(),
            function_id: record.function_id.clone(),
            namespace: record.namespace.clone(),
        },
    )
    .await?;

    let prompt = approval_prompt(&record);

    let keyboard = telegram::inline_keyboard(vec![
        vec![
            ("Approve".into(), format!("a:{token}")),
            ("Reject".into(), format!("d:{token}")),
        ],
        vec![("Approve always".into(), format!("w:{token}"))],
    ]);

    let message_id = telegram::send_message(deps, chat_id, &prompt, Some(keyboard), None).await?;
    kv::set_approval_message_id(
        deps,
        &record.session_id,
        &record.function_call_id,
        message_id,
    )
    .await?;
    Ok(BindingAck { ok: true })
}

fn short_token(function_call_id: &str) -> String {
    let mut hash: u64 = 0;
    for b in function_call_id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

pub(crate) fn approval_prompt(record: &PendingApprovalRecord) -> String {
    let args = record
        .arguments_excerpt
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".into());
    let ns = record
        .namespace
        .as_deref()
        .map(|n| format!(" in namespace `{n}`"))
        .unwrap_or_default();
    format!(
        "Approval required for `{}`{ns}\n{}",
        record.function_id,
        crate::text::truncate_ellipsis(&args, 500)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_prompt_names_the_namespace_when_present() {
        let mut r: PendingApprovalRecord = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "function_call_id": "c1", "function_id": "shell::exec", "arguments_excerpt": {"cmd": "ls"} })).unwrap();
        assert_eq!(
            approval_prompt(&r),
            "Approval required for `shell::exec`\n{\"cmd\":\"ls\"}"
        );
        r.namespace = Some("laptop".into());
        assert_eq!(
            approval_prompt(&r),
            "Approval required for `shell::exec` in namespace `laptop`\n{\"cmd\":\"ls\"}"
        );
    }
}
