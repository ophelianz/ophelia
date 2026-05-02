use super::host::OpheliaRequest;
use super::*;

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct LocalServiceOptions {
    pub service_binary: Option<PathBuf>,
    pub repair_policy: LocalServiceRepairPolicy,
    pub startup_timeout: Duration,
}

impl Default for LocalServiceOptions {
    fn default() -> Self {
        Self {
            service_binary: None,
            repair_policy: LocalServiceRepairPolicy::RepairIfSafe,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalServiceRepairPolicy {
    RepairIfSafe,
    WarnOnly,
}

#[derive(Clone)]
pub struct LocalServiceConnection {
    pub client: OpheliaClient,
    pub warning: Option<LocalServiceWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalServiceWarning {
    ActiveServiceMismatch {
        expected_binary: PathBuf,
        actual_binary: Option<PathBuf>,
        expected_version: String,
        actual_version: String,
    },
}

pub struct OpheliaSubscription {
    pub snapshot: OpheliaSnapshot,
    inner: OpheliaSubscriptionInner,
}

enum OpheliaSubscriptionInner {
    InProcess {
        events: broadcast::Receiver<OpheliaEvent>,
    },
    #[cfg(target_os = "macos")]
    Mach { events: super::xpc::MachEventStream },
}

impl OpheliaSubscription {
    pub(super) fn in_process(
        snapshot: OpheliaSnapshot,
        events: broadcast::Receiver<OpheliaEvent>,
    ) -> Self {
        Self {
            snapshot,
            inner: OpheliaSubscriptionInner::InProcess { events },
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn mach(snapshot: OpheliaSnapshot, events: super::xpc::MachEventStream) -> Self {
        Self {
            snapshot,
            inner: OpheliaSubscriptionInner::Mach { events },
        }
    }

    pub async fn next_event(&mut self) -> Result<OpheliaEvent, OpheliaError> {
        match &mut self.inner {
            OpheliaSubscriptionInner::InProcess { events } => {
                events.recv().await.map_err(|error| match error {
                    broadcast::error::RecvError::Closed => OpheliaError::Closed,
                    broadcast::error::RecvError::Lagged(skipped) => {
                        OpheliaError::Lagged { skipped }
                    }
                })
            }
            #[cfg(target_os = "macos")]
            OpheliaSubscriptionInner::Mach { events } => events.next_event().await,
        }
    }
}

#[derive(Clone)]
pub struct OpheliaClient {
    inner: Arc<OpheliaClientInner>,
}

pub(super) enum OpheliaClientInner {
    InProcess {
        tx: mpsc::Sender<OpheliaRequest>,
    },
    #[cfg(target_os = "macos")]
    Mach {
        next_id: AtomicU64,
    },
}

impl OpheliaClient {
    pub(super) fn in_process(tx: mpsc::Sender<OpheliaRequest>) -> Self {
        Self {
            inner: Arc::new(OpheliaClientInner::InProcess { tx }),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn connect_local() -> Result<Self, OpheliaError> {
        Ok(Self {
            inner: Arc::new(OpheliaClientInner::Mach {
                next_id: AtomicU64::new(1),
            }),
        })
    }

    #[cfg(target_os = "macos")]
    pub fn connect_or_start_local(
        options: LocalServiceOptions,
    ) -> Result<LocalServiceConnection, OpheliaError> {
        super::macos_startup::connect_or_start_local(options)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn connect_or_start_local(
        _: LocalServiceOptions,
    ) -> Result<LocalServiceConnection, OpheliaError> {
        Err(OpheliaError::Transport {
            message: "OpheliaService startup is only implemented on macOS".into(),
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn connect_local() -> Result<Self, OpheliaError> {
        Err(OpheliaError::Transport {
            message: "OpheliaService external transport is only implemented on macOS".into(),
        })
    }

    pub async fn add(&self, request: TransferRequest) -> Result<TransferId, OpheliaError> {
        match self.dispatch(OpheliaCommand::Add { request }).await? {
            OpheliaResponse::TransferAdded { id } => Ok(id),
            _ => Err(unexpected_response("add")),
        }
    }

    pub async fn pause(&self, id: TransferId) -> Result<(), OpheliaError> {
        self.expect_ack(OpheliaCommand::Pause { id }).await
    }

    pub async fn resume(&self, id: TransferId) -> Result<(), OpheliaError> {
        self.expect_ack(OpheliaCommand::Resume { id }).await
    }

    pub async fn cancel(&self, id: TransferId) -> Result<(), OpheliaError> {
        self.expect_ack(OpheliaCommand::Cancel { id }).await
    }

    pub async fn delete_artifact(&self, id: TransferId) -> Result<(), OpheliaError> {
        self.expect_ack(OpheliaCommand::DeleteArtifact { id }).await
    }

    pub async fn update_settings(&self, settings: ServiceSettings) -> Result<(), OpheliaError> {
        self.expect_ack(OpheliaCommand::UpdateSettings { settings })
            .await
    }

    pub async fn load_history(
        &self,
        filter: HistoryFilter,
        query: impl Into<String>,
    ) -> Result<Vec<HistoryRow>, OpheliaError> {
        match self
            .dispatch(OpheliaCommand::LoadHistory {
                filter,
                query: query.into(),
            })
            .await?
        {
            OpheliaResponse::History { rows } => Ok(rows),
            _ => Err(unexpected_response("load_history")),
        }
    }

    pub async fn service_info(&self) -> Result<OpheliaServiceInfo, OpheliaError> {
        match self.dispatch(OpheliaCommand::ServiceInfo).await? {
            OpheliaResponse::ServiceInfo { info } => Ok(info),
            _ => Err(unexpected_response("service_info")),
        }
    }

    pub async fn snapshot(&self) -> Result<OpheliaSnapshot, OpheliaError> {
        match self.dispatch(OpheliaCommand::Snapshot).await? {
            OpheliaResponse::Snapshot { snapshot } => Ok(snapshot),
            _ => Err(unexpected_response("snapshot")),
        }
    }

    pub async fn subscribe(&self) -> Result<OpheliaSubscription, OpheliaError> {
        match &*self.inner {
            OpheliaClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(OpheliaRequest::Subscribe { reply: reply_tx })
                    .await
                    .map_err(|_| OpheliaError::Closed)?;
                reply_rx.await.map_err(|_| OpheliaError::Closed)?
            }
            #[cfg(target_os = "macos")]
            OpheliaClientInner::Mach { next_id } => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let (snapshot, events) = super::xpc::subscribe_mach(id).await?;
                Ok(OpheliaSubscription::mach(snapshot, events))
            }
        }
    }

    pub(super) async fn shutdown_embedded(&self) -> Result<(), OpheliaError> {
        match &*self.inner {
            OpheliaClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(OpheliaRequest::Shutdown { reply: reply_tx })
                    .await
                    .map_err(|_| OpheliaError::Closed)?;
                match reply_rx.await.map_err(|_| OpheliaError::Closed)?? {
                    OpheliaResponse::Ack => Ok(()),
                    _ => Err(unexpected_response("shutdown")),
                }
            }
            #[cfg(target_os = "macos")]
            OpheliaClientInner::Mach { .. } => Err(OpheliaError::BadRequest {
                message: "shutdown is only available to the embedded service owner".into(),
            }),
        }
    }

    pub(super) fn try_shutdown_embedded(&self) {
        if let OpheliaClientInner::InProcess { tx } = &*self.inner {
            let _ = tx.try_send(OpheliaRequest::Shutdown {
                reply: oneshot::channel().0,
            });
        }
    }

    async fn expect_ack(&self, command: OpheliaCommand) -> Result<(), OpheliaError> {
        match self.dispatch(command).await? {
            OpheliaResponse::Ack => Ok(()),
            _ => Err(unexpected_response("ack")),
        }
    }

    pub(super) async fn dispatch(
        &self,
        command: OpheliaCommand,
    ) -> Result<OpheliaResponse, OpheliaError> {
        match &*self.inner {
            OpheliaClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(OpheliaRequest::Command {
                    command,
                    reply: reply_tx,
                })
                .await
                .map_err(|_| OpheliaError::Closed)?;
                reply_rx.await.map_err(|_| OpheliaError::Closed)?
            }
            #[cfg(target_os = "macos")]
            OpheliaClientInner::Mach { next_id } => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                super::xpc::dispatch_mach(id, command).await
            }
        }
    }
}

fn unexpected_response(command: &str) -> OpheliaError {
    OpheliaError::BadRequest {
        message: format!("unexpected response for {command}"),
    }
}
