//! The live updates websocket. The panel opens one socket and follows topics on it: each `sub`
//! checks the topic's permission against the session as it stands, then sends the topic's
//! snapshot and its events as they come. The socket closes as soon as its session ends.

mod topics;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures_util::stream::SplitStream;
use futures_util::{Sink, SinkExt, StreamExt};
use jiff::Timestamp;
use observer_app::Caller;
use observer_domain::error::ErrorCode;
use observer_domain::live::{
    CLOSE_SESSION_ENDED, CONTRACT, ClientMessage, PROTOCOL, ServerMessage, Topic, UnsubscribeReason,
};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};
use tokio::time::{Instant, interval_at, sleep, sleep_until, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::error;

use crate::AppState;
use crate::error::Failure;
use topics::{Delivery, Events};

/// Frames waiting for one socket. A client that lets this many pile up reads too slowly to follow.
const QUEUE_FRAMES: usize = 256;
/// What those frames may weigh together. A console snapshot alone runs to megabytes, so a client
/// that subscribes again and again without reading would otherwise make Observer hold gigabytes.
const QUEUE_BYTES: usize = 16 * 1024 * 1024;
/// Frames are written together, at most this often, rather than one write each.
const BATCH_EVERY: Duration = Duration::from_millis(50);
const PING_EVERY: Duration = Duration::from_secs(20);
/// A socket that sends nothing for this long, not even a pong, is dropped.
const SILENCE_LIMIT: Duration = Duration::from_secs(60);
/// A write that takes longer means the client stopped reading.
const WRITE_LIMIT: Duration = Duration::from_secs(10);
/// How long a closing socket gets to send what it still has.
const CLOSE_LIMIT: Duration = Duration::from_secs(2);
/// What the panel sends is a few dozen bytes. A frame announcing more is refused from its header,
/// before its payload is buffered.
const MAX_MESSAGE_BYTES: usize = 4096;
/// One per tab of the panel, which a browser shares a session between.
const SOCKETS_PER_SESSION: usize = 8;
const CLOSE_GOING_AWAY: u16 = 1001;
const CLOSE_INTERNAL_ERROR: u16 = 1011;

/// Every open socket, so that Observer can close them when it stops.
#[derive(Clone)]
pub struct Live {
    stop: CancellationToken,
    sockets: TaskTracker,
    connections: Arc<AtomicU64>,
    /// How many sockets each session holds open.
    open: Arc<Mutex<HashMap<i64, usize>>>,
    started_at: Timestamp,
}

impl Live {
    pub(crate) fn new() -> Self {
        Self {
            stop: CancellationToken::new(),
            sockets: TaskTracker::new(),
            connections: Arc::default(),
            open: Arc::default(),
            started_at: Timestamp::now(),
        }
    }

    /// Closes every socket as going away, which tells the panel to reconnect later, and returns
    /// once they are all closed.
    pub async fn shutdown(&self) {
        self.stop.cancel();
        self.sockets.close();
        self.sockets.wait().await;
    }

    /// A place for one more socket of the session, unless it holds as many as it may.
    fn place(&self, session_id: i64) -> Option<Place> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(session_id).or_default();
        if *count >= SOCKETS_PER_SESSION {
            return None;
        }
        *count += 1;
        Some(Place {
            open: Arc::clone(&self.open),
            session_id,
        })
    }
}

/// A socket counted against its session until it ends, or until its upgrade fails.
struct Place {
    open: Arc<Mutex<HashMap<i64, usize>>>,
    session_id: i64,
}

impl Drop for Place {
    fn drop(&mut self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Entry::Occupied(mut count) = open.entry(self.session_id) {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
    }
}

/// `GET /api/ws`, once its gate has spent the ticket.
pub(crate) async fn open(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Ok(upgrade) = upgrade else {
        return Failure::from(ErrorCode::InvalidRequest).into_response();
    };
    if !upgrade
        .requested_protocols()
        .any(|protocol| protocol.as_bytes() == PROTOCOL.as_bytes())
    {
        return Failure::from(ErrorCode::InvalidRequest).into_response();
    }
    let live = state.live.clone();
    let Some(place) = live.place(caller.session_id) else {
        return Failure::from(ErrorCode::TooManySockets).into_response();
    };
    let id = live.connections.fetch_add(1, Ordering::Relaxed) + 1;
    // An idle socket then costs a few kilobytes rather than the 128 KiB read buffer it would
    // otherwise reserve up front.
    upgrade
        .protocols([PROTOCOL])
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .read_buffer_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| live.sockets.track_future(serve(socket, state, caller, id, place)))
}

async fn serve(socket: WebSocket, state: AppState, caller: Caller, id: u64, _place: Place) {
    let (sink, mut inbound) = socket.split();
    let (queue, outbound) = Queue::new(QUEUE_FRAMES, QUEUE_BYTES);
    let writer = tokio::spawn(write(sink, outbound, Arc::clone(&queue.held)));
    let mut connection = Connection {
        state,
        caller,
        queue,
        followed: HashMap::new(),
        forwarders: JoinSet::new(),
    };
    let end = connection.run(&mut inbound, id).await;
    connection.end(end, writer).await;
}

/// How a socket ends: with a close frame, or dropped, when the client is gone or too slow to
/// take one.
enum End {
    Close(u16),
    Drop,
}

struct Connection {
    state: AppState,
    /// Rechecked on every `sub` and whenever sessions change, so its permissions are current.
    caller: Caller,
    queue: Queue,
    /// The topics followed, with the task forwarding their events when they have any.
    followed: HashMap<Topic, Option<AbortHandle>>,
    forwarders: JoinSet<()>,
}

impl Connection {
    async fn run(&mut self, inbound: &mut SplitStream<WebSocket>, id: u64) -> End {
        let hello = ServerMessage::Hello {
            connection_id: id,
            contract: CONTRACT,
        };
        if let Err(end) = self.send(&hello) {
            return end;
        }
        let stop = self.state.live.stop.clone();
        let mut sessions = self.state.accounts.session_changes();
        let mut ping = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
        let mut silent_until = Instant::now() + SILENCE_LIMIT;

        loop {
            let outcome = tokio::select! {
                () = stop.cancelled() => Err(End::Close(CLOSE_GOING_AWAY)),
                () = sleep_until(silent_until) => Err(End::Drop),
                frame = inbound.next() => match frame {
                    Some(Ok(frame)) => {
                        silent_until = Instant::now() + SILENCE_LIMIT;
                        self.receive(frame).await
                    }
                    // The client left, or broke the protocol.
                    _ => Err(End::Drop),
                },
                Ok(()) = sessions.changed() => self.recheck().await,
                _ = ping.tick() => match self.queue.offer(Message::Ping(Bytes::new())) {
                    Ok(()) => self.recheck().await,
                    Err(_) => Err(End::Drop),
                },
            };
            if let Err(end) = outcome {
                return end;
            }
        }
    }

    async fn receive(&mut self, frame: Message) -> Result<(), End> {
        let text = match frame {
            Message::Text(text) => text,
            Message::Binary(_) => {
                return self.send(&ServerMessage::Error {
                    code: ErrorCode::InvalidRequest,
                });
            }
            Message::Close(_) => return Err(End::Drop),
            Message::Ping(_) | Message::Pong(_) => return Ok(()),
        };
        match serde_json::from_str::<ClientMessage>(text.as_str()) {
            Ok(ClientMessage::Sub { topic, since }) => self.follow(topic, since).await,
            Ok(ClientMessage::Unsub { topic }) => {
                self.unfollow(topic);
                Ok(())
            }
            Ok(ClientMessage::Ping) => self.send(&ServerMessage::Pong),
            Err(_) => self.send(&ServerMessage::Error {
                code: ErrorCode::InvalidRequest,
            }),
        }
    }

    /// Starts a topic over: its permission against the session as it is now, then its snapshot,
    /// then its events.
    async fn follow(&mut self, topic: Topic, since: Option<u64>) -> Result<(), End> {
        self.recheck().await?;
        self.unfollow(topic);
        if !topics::allows(topic, self.caller.permissions) {
            return self.send(&ServerMessage::Unsubscribed {
                topic,
                reason: UnsubscribeReason::Forbidden,
            });
        }
        let feed = match topics::open(topic, since, &self.state).await {
            Ok(feed) => feed,
            Err(failure) => {
                error!(error = %failure, ?topic, "a live topic could not start");
                return self.send(&ServerMessage::Error {
                    code: ErrorCode::Internal,
                });
            }
        };
        self.send(&ServerMessage::Snapshot(feed.snapshot))?;
        while self.forwarders.try_join_next().is_some() {}
        let forwarder = feed
            .events
            .map(|events| self.forwarders.spawn(forward(topic, events, self.queue.clone())));
        self.followed.insert(topic, forwarder);
        Ok(())
    }

    fn unfollow(&mut self, topic: Topic) {
        if let Some(Some(forwarder)) = self.followed.remove(&topic) {
            forwarder.abort();
        }
    }

    /// Ends the socket once its session has, and stops the topics whose permission went with a
    /// change of the user's.
    async fn recheck(&mut self) -> Result<(), End> {
        match self.state.accounts.recheck(&self.caller).await {
            Ok(Some(caller)) => self.caller = caller,
            Ok(None) => return Err(End::Close(CLOSE_SESSION_ENDED)),
            Err(failure) => {
                error!(error = %failure, "a live socket could not recheck its session");
                return Err(End::Close(CLOSE_INTERNAL_ERROR));
            }
        }
        let withdrawn: Vec<Topic> = self
            .followed
            .keys()
            .copied()
            .filter(|&topic| !topics::allows(topic, self.caller.permissions))
            .collect();
        for topic in withdrawn {
            self.unfollow(topic);
            self.send(&ServerMessage::Unsubscribed {
                topic,
                reason: UnsubscribeReason::Forbidden,
            })?;
        }
        Ok(())
    }

    /// Queues a message of the socket's own. A client that cannot take even those is dropped.
    fn send(&self, message: &ServerMessage) -> Result<(), End> {
        self.queue.offer(text(message)).map_err(|_| End::Drop)
    }

    async fn end(mut self, end: End, writer: JoinHandle<()>) {
        self.forwarders.shutdown().await;
        if let End::Close(code) = end {
            let close = Message::Close(Some(CloseFrame {
                code,
                reason: Utf8Bytes::default(),
            }));
            let _ = timeout(CLOSE_LIMIT, self.queue.send(close)).await;
        }
        drop(self);
        let abort = writer.abort_handle();
        if timeout(CLOSE_LIMIT, writer).await.is_err() {
            abort.abort();
        }
    }
}

/// Hands a topic's events to the socket until they end. When the socket's queue is full, the
/// client reads slower than events come: rather than keep them for it, or slow anyone else
/// down, the topic stops with `resync`, and the client starts over from a fresh snapshot.
async fn forward(topic: Topic, mut events: Events, queue: Queue) {
    loop {
        match events.next().await {
            Some(Delivery::Event(event)) => match queue.offer(text(&ServerMessage::Event(*event))) {
                Ok(()) => {}
                Err(Refused::Full) => break,
                Err(Refused::Closed) => return,
            },
            Some(Delivery::Missed) => break,
            None => return,
        }
    }
    let _ = queue.send(text(&ServerMessage::Resync { topic })).await;
}

/// A socket's outgoing frames, bounded by their count and by what they weigh. A frame weighs on
/// the queue from the moment it is offered until the writer has flushed it to the client.
#[derive(Clone)]
struct Queue {
    frames: mpsc::Sender<Message>,
    held: Arc<AtomicUsize>,
    max_bytes: usize,
}

enum Refused {
    /// The client reads slower than frames come.
    Full,
    /// The writer is gone, and the socket with it.
    Closed,
}

impl Queue {
    fn new(max_frames: usize, max_bytes: usize) -> (Self, mpsc::Receiver<Message>) {
        let (frames, outbound) = mpsc::channel(max_frames);
        let queue = Self {
            frames,
            held: Arc::default(),
            max_bytes,
        };
        (queue, outbound)
    }

    /// Queues the frame unless the queue is full. An empty queue takes a frame of any size, so
    /// that a snapshot heavier than the whole budget still goes out alone.
    fn offer(&self, message: Message) -> Result<(), Refused> {
        let size = weight(&message);
        let before = self.held.fetch_add(size, Ordering::AcqRel);
        if before > 0 && before + size > self.max_bytes {
            self.held.fetch_sub(size, Ordering::AcqRel);
            return Err(Refused::Full);
        }
        self.frames.try_send(message).map_err(|error| {
            self.held.fetch_sub(size, Ordering::AcqRel);
            match error {
                TrySendError::Full(_) => Refused::Full,
                TrySendError::Closed(_) => Refused::Closed,
            }
        })
    }

    /// Queues a frame of the socket's own, a resync or a close, once there is room for it.
    async fn send(&self, message: Message) -> Result<(), Refused> {
        let size = weight(&message);
        self.held.fetch_add(size, Ordering::AcqRel);
        self.frames.send(message).await.map_err(|_| {
            self.held.fetch_sub(size, Ordering::AcqRel);
            Refused::Closed
        })
    }
}

fn weight(message: &Message) -> usize {
    match message {
        Message::Text(text) => text.len(),
        Message::Binary(bytes) | Message::Ping(bytes) | Message::Pong(bytes) => bytes.len(),
        Message::Close(_) => 0,
    }
}

/// Writes what the socket's queue holds, a batch at a time, and takes what it wrote off the
/// queue's weight.
async fn write(
    mut sink: impl Sink<Message> + Unpin,
    mut outbound: mpsc::Receiver<Message>,
    held: Arc<AtomicUsize>,
) {
    while let Some(first) = outbound.recv().await {
        let mut closing = false;
        let mut written = 0;
        let batch = timeout(WRITE_LIMIT, async {
            let mut next = Some(first);
            while let Some(message) = next.take() {
                closing = matches!(message, Message::Close(_));
                written += weight(&message);
                sink.feed(message).await?;
                if !closing {
                    next = outbound.try_recv().ok();
                }
            }
            sink.flush().await
        })
        .await;
        held.fetch_sub(written, Ordering::AcqRel);
        if closing || !matches!(batch, Ok(Ok(()))) {
            return;
        }
        sleep(BATCH_EVERY).await;
    }
    let _ = sink.close().await;
}

fn text(message: &ServerMessage) -> Message {
    let json = serde_json::to_string(message).expect("live messages have string keys only");
    Message::Text(json.into())
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, Outcome};
    use observer_domain::live::TopicEvent;
    use serde_json::{Value, json};

    use super::*;

    fn entry(id: i64) -> Delivery {
        Delivery::Event(Box::new(TopicEvent::Audit(AuditEntry {
            id,
            at: "2026-10-28T09:00:00Z".parse().unwrap(),
            actor_kind: ActorKind::Cli,
            actor_id: None,
            actor_name: None,
            ip: None,
            user_agent: None,
            action: AuditAction::UserUnlocked,
            target: Some("grace".to_owned()),
            outcome: Outcome::Success,
            details: json!({}),
        })))
    }

    fn json(frame: Message) -> Value {
        let Message::Text(text) = frame else {
            panic!("expected a text frame, got {frame:?}")
        };
        serde_json::from_str(text.as_str()).unwrap()
    }

    #[tokio::test]
    async fn a_full_queue_drops_the_rest_and_asks_for_a_resync_once_there_is_room() {
        let (queue, mut frames) = Queue::new(2, QUEUE_BYTES);
        let events: Events = Box::pin(stream::iter((1..=5).map(entry)));

        let forwarder = tokio::spawn(forward(Topic::Audit, events, queue));
        tokio::task::yield_now().await;

        assert_eq!(json(frames.recv().await.unwrap())["data"]["id"], 1);
        assert_eq!(json(frames.recv().await.unwrap())["data"]["id"], 2);
        assert_eq!(
            json(frames.recv().await.unwrap()),
            json!({ "type": "resync", "topic": "audit" })
        );
        forwarder.await.unwrap();
        assert!(frames.recv().await.is_none(), "events 3 to 5 are gone");
    }

    #[tokio::test]
    async fn a_topic_whose_events_were_lost_upstream_asks_for_a_resync() {
        let (queue, mut frames) = Queue::new(8, QUEUE_BYTES);
        let events: Events = Box::pin(stream::iter([entry(1), Delivery::Missed, entry(3)]));

        forward(Topic::Audit, events, queue).await;

        assert_eq!(json(frames.recv().await.unwrap())["data"]["id"], 1);
        assert_eq!(json(frames.recv().await.unwrap())["type"], "resync");
        assert!(frames.recv().await.is_none());
    }

    /// A console snapshot of 4 MiB, the size of the whole ring.
    fn snapshot() -> Message {
        Message::Text("x".repeat(4 * 1024 * 1024).into())
    }

    #[test]
    fn a_client_that_never_reads_holds_its_budget_however_often_it_subscribes() {
        let (queue, _unread) = Queue::new(QUEUE_FRAMES, QUEUE_BYTES);

        let taken = (0..QUEUE_FRAMES)
            .take_while(|_| queue.offer(snapshot()).is_ok())
            .count();

        assert_eq!(taken, 4, "16 MiB of 4 MiB snapshots, not {QUEUE_FRAMES} of them");
        assert_eq!(queue.held.load(Ordering::Acquire), QUEUE_BYTES);
        assert!(
            matches!(queue.offer(text(&ServerMessage::Pong)), Err(Refused::Full)),
            "even a small frame waits for the client to read"
        );
    }

    #[test]
    fn an_empty_queue_takes_a_snapshot_heavier_than_its_budget() {
        let (queue, _unread) = Queue::new(QUEUE_FRAMES, 1024 * 1024);

        assert!(queue.offer(snapshot()).is_ok());
        assert!(matches!(queue.offer(snapshot()), Err(Refused::Full)));
    }

    #[tokio::test]
    async fn frames_weigh_on_the_queue_until_they_are_written() {
        let (queue, frames) = Queue::new(QUEUE_FRAMES, QUEUE_BYTES);
        while queue.offer(snapshot()).is_ok() {}

        let writer = tokio::spawn(write(
            futures_util::sink::drain(),
            frames,
            Arc::clone(&queue.held),
        ));
        timeout(Duration::from_secs(5), async {
            while queue.held.load(Ordering::Acquire) > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the written frames still weigh on the queue");

        assert!(queue.offer(snapshot()).is_ok());
        writer.abort();
    }
}
