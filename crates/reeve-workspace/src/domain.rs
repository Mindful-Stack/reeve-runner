//! Workspace + Container domain entities.

use reeve_protocol::ContainerStatus;
use std::path::PathBuf;

pub struct Workspace {
    card_id: String,
    container: Option<Container>,
    socket_path: PathBuf,
}

pub struct Container {
    name: String,
    status: ContainerStatus,
    started_at: i64,
}

impl Workspace {
    pub fn for_card(card_id: String, socket_path: PathBuf) -> Self {
        Self {
            card_id,
            container: None,
            socket_path,
        }
    }

    pub fn card_id(&self) -> &str {
        &self.card_id
    }
    pub fn container(&self) -> Option<&Container> {
        self.container.as_ref()
    }
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }
    pub fn is_running(&self) -> bool {
        matches!(
            self.container.as_ref().map(|c| c.status),
            Some(ContainerStatus::Running)
        )
    }

    pub fn with_container(self, container: Container) -> Self {
        Self {
            container: Some(container),
            ..self
        }
    }
    pub fn without_container(self) -> Self {
        Self {
            container: None,
            ..self
        }
    }
}

impl Container {
    pub fn new(name: String, status: ContainerStatus, started_at: i64) -> Self {
        Self {
            name,
            status,
            started_at,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn status(&self) -> ContainerStatus {
        self.status
    }
    pub fn started_at(&self) -> i64 {
        self.started_at
    }
    pub fn with_status(self, status: ContainerStatus) -> Self {
        Self { status, ..self }
    }
}
