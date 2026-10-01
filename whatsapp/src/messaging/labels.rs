use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelRequest {
    pub session_id: String,
    pub label_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLabelRequest {
    pub session_id: String,
    pub label_id: String,
    pub name: String,
    pub color: i32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatLabelRequest {
    pub session_id: String,
    pub label_id: String,
    pub chat_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageLabelRequest {
    pub session_id: String,
    pub label_id: String,
    pub chat_id: String,
    pub message_id: String,
}

pub(super) fn register(registry: &mut Registry) {
    registry.add::<SetLabelRequest, Applied, _, _>(
        "whatsapp::labels::set",
        "Create or update a label by its stable id.",
        |m, r| async move {
            validate::text(&r.label_id, "label_id", 128)?;
            validate::text(&r.name, "name", 100)?;
            ensure!(
                (0..=19).contains(&r.color),
                "label color must be between 0 and 19"
            );
            m.client(&r.session_id)
                .await?
                .labels()
                .create_label(&r.label_id, &r.name, r.color)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<LabelRequest, Applied, _, _>(
        "whatsapp::labels::delete",
        "Delete a label by id.",
        |m, r| async move {
            validate::text(&r.label_id, "label_id", 128)?;
            m.client(&r.session_id)
                .await?
                .labels()
                .delete_label(&r.label_id)
                .await?;
            Ok(applied())
        },
    );
    macro_rules! chat_label {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<ChatLabelRequest, Applied, _, _>($id, $description, |m, r| async move {
                validate::text(&r.label_id, "label_id", 128)?;
                let chat = validate::jid(&r.chat_id)?;
                m.client(&r.session_id)
                    .await?
                    .labels()
                    .$method(&r.label_id, &chat)
                    .await?;
                Ok(applied())
            });
        };
    }
    chat_label!(
        "whatsapp::labels::add_chat",
        "Associate a label with a chat.",
        add_chat_label
    );
    chat_label!(
        "whatsapp::labels::remove_chat",
        "Remove a label from a chat.",
        remove_chat_label
    );
    macro_rules! message_label {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<MessageLabelRequest, Applied, _, _>(
                $id,
                $description,
                |m, r| async move {
                    validate::text(&r.label_id, "label_id", 128)?;
                    message_id(&r.message_id)?;
                    let chat = validate::jid(&r.chat_id)?;
                    m.client(&r.session_id)
                        .await?
                        .labels()
                        .$method(&r.label_id, &chat, &r.message_id)
                        .await?;
                    Ok(applied())
                },
            );
        };
    }
    message_label!(
        "whatsapp::labels::add_message",
        "Associate a label with a message using the native legacy app-state mutation.",
        add_message_label
    );
    message_label!(
        "whatsapp::labels::remove_message",
        "Remove a message label using the native legacy app-state mutation.",
        remove_message_label
    );
}
