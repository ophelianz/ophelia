use super::ffi::{
    XpcConnection, XpcObjectRaw, frame_from_xpc_event, message_from_body, xpc_connection_activate,
    xpc_connection_send_message_with_reply, xpc_connection_set_event_handler,
};
use crate::service::codec::{
    OpheliaCommandEnvelope, OpheliaFrameEnvelope, command_to_body, unexpected_xpc_frame,
};
use crate::service::{OpheliaCommand, OpheliaError, OpheliaResponse};
use block2::RcBlock;
use futures::channel::oneshot as futures_oneshot;
use std::ptr;
use std::sync::{OnceLock, mpsc as std_mpsc};
use std::time::Duration;

const MACH_COMMAND_QUEUE_CAPACITY: usize = 64;

pub(in crate::service) async fn dispatch_mach(
    id: u64,
    command: OpheliaCommand,
) -> Result<OpheliaResponse, OpheliaError> {
    mach_command_pool()?
        .run(move || dispatch_mach_blocking(id, command))
        .await
}

fn dispatch_mach_blocking(
    id: u64,
    command: OpheliaCommand,
) -> Result<OpheliaResponse, OpheliaError> {
    let connection = XpcConnection::connect_client()?;
    let noop = RcBlock::new(|_event: XpcObjectRaw| {});
    unsafe {
        xpc_connection_set_event_handler(connection.raw(), &noop);
        xpc_connection_activate(connection.raw());
    }

    let command = OpheliaCommandEnvelope { id, command };
    let message = message_from_body(&command_to_body(&command)?)?;
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let reply_handler = RcBlock::new(move |reply: XpcObjectRaw| {
        let _ = reply_tx.send(frame_from_xpc_event(reply));
    });
    unsafe {
        xpc_connection_send_message_with_reply(
            connection.raw(),
            message.raw(),
            ptr::null_mut(),
            &reply_handler,
        );
    }
    let frame = reply_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| OpheliaError::Closed)??;

    match frame {
        OpheliaFrameEnvelope::Response {
            id: frame_id,
            response,
        } if frame_id == id => Ok(*response),
        OpheliaFrameEnvelope::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_xpc_frame("response", frame)),
    }
}

struct BlockingCommandPool {
    tx: std_mpsc::SyncSender<BlockingCommandJob>,
}

struct BlockingCommandJob {
    work: Box<dyn FnOnce() -> Result<OpheliaResponse, OpheliaError> + Send>,
    reply: futures_oneshot::Sender<Result<OpheliaResponse, OpheliaError>>,
}

impl BlockingCommandPool {
    fn start() -> Result<Self, OpheliaError> {
        let (tx, rx) = std_mpsc::sync_channel::<BlockingCommandJob>(MACH_COMMAND_QUEUE_CAPACITY);
        start_command_worker(rx)?;
        Ok(Self { tx })
    }

    async fn run<F>(&self, work: F) -> Result<OpheliaResponse, OpheliaError>
    where
        F: FnOnce() -> Result<OpheliaResponse, OpheliaError> + Send + 'static,
    {
        // GPUI futures are not Tokio tasks, so Mach commands use this bounded worker instead
        let (reply, rx) = futures_oneshot::channel();
        let job = BlockingCommandJob {
            work: Box::new(work),
            reply,
        };
        self.tx.try_send(job).map_err(command_queue_error)?;

        rx.await.map_err(|_| OpheliaError::Transport {
            message: "Mach command worker exited without a response".into(),
        })?
    }
}

fn start_command_worker(rx: std_mpsc::Receiver<BlockingCommandJob>) -> Result<(), OpheliaError> {
    std::thread::Builder::new()
        .name("ophelia-xpc-command".to_string())
        .spawn(move || run_command_worker(rx))
        .map(|_| ())
        .map_err(|error| OpheliaError::Transport {
            message: format!("failed to spawn Mach command worker: {error}"),
        })
}

fn run_command_worker(rx: std_mpsc::Receiver<BlockingCommandJob>) {
    while let Ok(job) = rx.recv() {
        let result = run_command_job(job.work);
        let _ = job.reply.send(result);
    }
}

fn run_command_job(
    work: Box<dyn FnOnce() -> Result<OpheliaResponse, OpheliaError> + Send>,
) -> Result<OpheliaResponse, OpheliaError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| command_worker_panic_error())
}

fn command_worker_panic_error() -> Result<OpheliaResponse, OpheliaError> {
    Err(OpheliaError::Transport {
        message: "Mach command worker panicked".into(),
    })
}

fn command_queue_error(error: std_mpsc::TrySendError<BlockingCommandJob>) -> OpheliaError {
    match error {
        std_mpsc::TrySendError::Full(_) => OpheliaError::Transport {
            message: "Mach command queue is full".into(),
        },
        std_mpsc::TrySendError::Disconnected(_) => OpheliaError::Closed,
    }
}

static MACH_COMMAND_POOL: OnceLock<Result<BlockingCommandPool, String>> = OnceLock::new();

fn mach_command_pool() -> Result<&'static BlockingCommandPool, OpheliaError> {
    match MACH_COMMAND_POOL
        .get_or_init(|| BlockingCommandPool::start().map_err(|error| error.to_string()))
    {
        Ok(pool) => Ok(pool),
        Err(message) => Err(OpheliaError::Transport {
            message: message.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_command_pool_does_not_need_tokio_runtime() {
        let pool = BlockingCommandPool::start().unwrap();
        let response = futures::executor::block_on(pool.run(|| Ok(OpheliaResponse::Ack))).unwrap();

        assert!(matches!(response, OpheliaResponse::Ack));
    }
}
