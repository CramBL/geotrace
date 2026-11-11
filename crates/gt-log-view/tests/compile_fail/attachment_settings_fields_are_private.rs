use gt_history_types::StoredLogSettings;
use gt_log_view::{
    AttachmentSettingsCompletion, AttachmentSettingsRevision, AttachmentSettingsWrite,
    LogAttachmentRef,
};
use uuid::Uuid;

fn forge_write(
    attachment: LogAttachmentRef,
    revision: AttachmentSettingsRevision,
    settings: StoredLogSettings,
) {
    let write = AttachmentSettingsWrite {
        attachment,
        revision,
        settings,
    };
    let _completion = AttachmentSettingsCompletion {
        write,
        result: Ok(()),
    };
    let _revision = AttachmentSettingsRevision {
        context: Uuid::nil(),
        revision: Uuid::nil(),
    };
}

fn main() {}
