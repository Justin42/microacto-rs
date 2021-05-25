use async_trait::async_trait;
use log::{error, info, trace, warn};
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};
use tokio::sync::Notify;
use tokio::time::Duration;

pub trait Context: Send + Sync {}

pub struct ActorContext<T>
where
    T: Actor,
{
    addr: WeakAddr<T>,
}

impl<T> Clone for ActorContext<T>
where
    T: Actor,
{
    fn clone(&self) -> Self {
        Self {
            addr: self.addr.clone(),
        }
    }
}

impl<T> ActorContext<T>
where
    T: Actor,
{
    pub fn build(addr: &Addr<T>) -> Self {
        Self {
            addr: addr.clone().downgrade(),
        }
    }

    pub fn upgrade_address(&self) -> Option<Addr<T>> {
        self.addr.clone().upgrade()
    }
}

#[derive(Clone, Debug)]
pub enum ActorError {
    RecvError,
    SendError,
    JoinError,
    Other(Arc<ActorErrorMessage>),
}

#[derive(Debug)]
pub struct ActorErrorMessage {
    inner: String,
}

impl ActorErrorMessage {
    pub fn new(inner: String) -> Self {
        Self { inner }
    }
}

impl std::convert::From<ActorErrorMessage> for ActorError {
    fn from(e: ActorErrorMessage) -> Self {
        Self::Other(Arc::new(e))
    }
}

impl std::convert::From<&str> for ActorError {
    fn from(msg: &str) -> Self {
        msg.to_string().into()
    }
}

impl std::convert::From<String> for ActorError {
    fn from(msg: String) -> Self {
        ActorErrorMessage::new(msg).into()
    }
}

impl<E: 'static> std::convert::From<E> for ActorErrorMessage
where
    E: std::error::Error,
{
    fn from(err: E) -> Self {
        Self {
            inner: err.to_string(),
        }
    }
}

impl std::error::Error for ActorError {}

impl Display for ActorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", &self)
    }
}

pub struct Envelope<A> {
    inner: Box<dyn ActorTask<A>>,
}

#[derive(Debug)]
pub struct Addr<A: Actor + 'static> {
    tx: Arc<tokio::sync::mpsc::UnboundedSender<Envelope<A>>>,
}

impl<A> Clone for Addr<A>
where
    A: Actor + 'static,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<A> Addr<A>
where
    A: Actor,
{
    pub fn downgrade(self) -> WeakAddr<A> {
        WeakAddr {
            tx: Arc::downgrade(&self.tx),
        }
    }

    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.tx)
    }

    pub fn weak_count(&self) -> usize {
        Arc::weak_count(&self.tx)
    }
}

pub struct WeakAddr<A>
where
    A: Actor,
{
    tx: Weak<tokio::sync::mpsc::UnboundedSender<Envelope<A>>>,
}

impl<A> WeakAddr<A>
where
    A: Actor,
{
    pub fn upgrade(&self) -> Option<Addr<A>> {
        Some(Addr {
            tx: self.tx.upgrade()?,
        })
    }
}

impl<A> Clone for WeakAddr<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<A> std::convert::From<tokio::sync::mpsc::UnboundedSender<Envelope<A>>> for Addr<A>
where
    A: Actor + 'static,
{
    fn from(tx: tokio::sync::mpsc::UnboundedSender<Envelope<A>>) -> Self {
        Self { tx: Arc::new(tx) }
    }
}

pub struct ActorTaskMessage<A, M>
where
    A: Actor + Handler<M>,
    M: Message,
{
    msg: Arc<M>,
    tx: Option<tokio::sync::oneshot::Sender<M::Result>>,
    marker: PhantomData<A>,
}

impl<A, M> ActorTaskMessage<A, M>
where
    A: Actor + Handler<M>,
    M: Message,
{
    pub fn new(msg: M, tx: Option<tokio::sync::oneshot::Sender<M::Result>>) -> Self {
        Self {
            msg: Arc::new(msg),
            tx,
            marker: Default::default(),
        }
    }
}

#[async_trait]
impl<A, M: 'static> ActorTask<A> for ActorTaskMessage<A, M>
where
    A: Actor + Handler<M>,
    M: Message,
{
    async fn execute(&mut self, actor: &A) {
        let tx = self.tx.take();
        match actor.handle(self.msg.clone()).await {
            Ok(r) => match tx {
                None => {}
                Some(s) => match s.send(r) {
                    Ok(_) => {
                        trace!("Responded with task result.");
                    }
                    Err(_) => {
                        warn!("Failed to deliver task result. Receiver dropped.")
                    }
                },
            },
            Err(e) => {
                error!("Error handling actor task: {:?}", e);
            }
        }
    }
}

impl<A, M: 'static> std::convert::From<ActorTaskMessage<A, M>> for Envelope<A>
where
    A: Actor + Handler<M> + 'static,
    M: Message,
{
    fn from(task: ActorTaskMessage<A, M>) -> Self {
        Envelope {
            inner: Box::new(task),
        }
    }
}

#[async_trait]
pub trait MessageSender<A, M>
where
    A: Actor + Handler<M>,
    M: Message + 'static,
{
    async fn send(&self, msg: M) -> Result<M::Result, ActorError>;
    fn notify(&self, msg: M);
    fn notify_later(&self, msg: M, duration: Duration);
}

#[async_trait]
impl<A, M> MessageSender<A, M> for Addr<A>
where
    A: Actor + Handler<M> + 'static,
    M: Message + 'static,
{
    async fn send(&self, msg: M) -> Result<<M as Message>::Result, ActorError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = ActorTaskMessage::<A, M>::new(msg, Some(tx));
        match self.tx.send(task.into()) {
            Ok(_) => match rx.await {
                Ok(v) => Ok(v),
                Err(e) => {
                    error!(
                        "Unable to receive actor task result from {} - {}: {:?}",
                        std::any::type_name::<A>(),
                        std::any::type_name::<M>(),
                        e.to_string()
                    );
                    Err(ActorError::RecvError)
                }
            },
            Err(e) => {
                error!(
                    "Unable to send actor task to {} - {}: {:?}",
                    std::any::type_name::<A>(),
                    std::any::type_name::<M>(),
                    e.to_string()
                );
                Err(ActorError::SendError)
            }
        }
    }

    fn notify(&self, msg: M) {
        let task = ActorTaskMessage::<A, M>::new(msg, None);
        match self.tx.send(task.into()) {
            Ok(_) => {}
            Err(e) => {
                error!(
                    "Unable to send actor task to {} - {}: {:?}",
                    std::any::type_name::<A>(),
                    std::any::type_name::<M>(),
                    e.to_string()
                );
            }
        }
    }

    fn notify_later(&self, msg: M, delay: Duration) {
        let addr = self.clone();
        tokio::task::spawn(async move {
            tokio::time::sleep(delay).await;
            addr.notify(msg);
        });
    }
}

#[async_trait]
pub trait ActorTask<A: Actor + Send + 'static>: Send + Sync + 'static {
    async fn execute(&mut self, actor: &A);
}

#[async_trait]
pub trait Actor: Send + Sized + Sync + 'static {
    fn start<A: Actor>(actor: A) -> Addr<A> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Envelope<A>>();
        let addr = Addr::from(tx);
        let ctx = ActorContext::build(&addr);

        {
            tokio::task::spawn(async move {
                let actor = actor;
                // TODO Sending a message to the ctx in on_start implementation will probably cause a deadlock.
                match actor.on_start(&ctx).await.err() {
                    None => {
                        while let Some(mut msg) = rx.recv().await {
                            msg.inner.execute(&actor).await
                        }
                        info!(
                            "Actor '{}' shutting down. All senders dropped.",
                            std::any::type_name::<A>()
                        );
                        match actor.on_stop(&ctx).await.err() {
                            None => {}
                            Some(e) => {
                                error!(
                                    "Error shutting down actor '{}': {:?}",
                                    std::any::type_name::<A>(),
                                    e
                                );
                            }
                        }
                    }
                    Some(e) => {
                        error!(
                            "Error starting actor {}: {:?}",
                            std::any::type_name::<A>(),
                            e
                        );
                    }
                }
            });
        }
        addr
    }

    async fn on_start(&self, _ctx: &ActorContext<Self>) -> Result<(), ActorError> {
        // TODO Don't do this. Message handling has not been started.
        // Spawn a new task or something instead (it wont complete until after handling starts)
        //ctx.addr.upgrade().unwrap().send(SomeMessage).await??;

        Ok(())
    }

    async fn on_stop(&self, _ctx: &ActorContext<Self>) -> Result<(), ActorError> {
        Ok(())
    }
}

pub trait Message: Send + Sync {
    type Result: Send;
}

#[async_trait]
pub trait Handler<M: Message> {
    async fn handle(&self, msg: Arc<M>) -> Result<M::Result, ActorError>;
}
