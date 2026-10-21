//! The slots writer's actions (`slot.*`).

use super::*;

impl Agent {
    /// The slot actions, through the slots writer (`update <command> <id>`).
    pub(super) async fn slot_action(self: &Arc<Self>, id: &str, request: &ActionRequest) {
        let Some(TypedConfigValue::String(slot)) = request.args.get("id") else {
            return self
                .reject(id, "`id` (string, the slot) is required".into())
                .await;
        };
        let slot = slot.clone();
        if self
            .writer
            .read_status()
            .is_some_and(|s| s.layout != "slots")
        {
            return self
                .reject(id, "this board has no slots (an A/B board)".into())
                .await;
        }
        let command = match request.name.as_str() {
            SLOT_BOOT_ONCE => "boot-once",
            SLOT_SET_DEFAULT => "set-default",
            SLOT_REMOVE => "remove",
            _ => match request.args.get("pinned") {
                Some(TypedConfigValue::Bool(false)) => "unpin",
                _ => "pin",
            },
        };
        if command == "boot-once" {
            // Report first: the restart ends this process and the action's record.
            self.succeed(id, phase(update_action::PHASE_REBOOTING))
                .await;
            let agent = self.clone();
            tokio::spawn(async move {
                if let Err(error) = agent.writer.run(&["boot-once", &slot]).await {
                    crate::log(&format!("boot-once {slot}: {error}"));
                }
                agent.publish(false).await;
            });
            return;
        }
        let result = {
            let _gate = self.apply_gate.lock().await;
            self.writer.run(&[command, &slot]).await
        };
        self.publish(false).await;
        match result {
            Ok(_) => self.succeed(id, BTreeMap::new()).await,
            Err(error) => self.reject(id, error).await,
        }
    }
}
