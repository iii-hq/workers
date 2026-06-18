use std::sync::Arc;

use iii_sdk::{IIIError, III};

use crate::clients::telegram;
use crate::deps::Deps;
use crate::kv;
use crate::types::PendingApprovalRecord;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::super::register(
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
) -> Result<BindingAck, IIIError> {
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
        },
    )
    .await?;

    let args = record
        .arguments_excerpt
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".into());
    let prompt = format!(
        "Approval required for `{}`\n{}",
        record.function_id,
        truncate(&args, 500)
    );

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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
