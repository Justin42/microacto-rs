use async_trait::async_trait;
use log::{error, info, trace, warn};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

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
            addr: addr.downgrade(),
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
    inner: Box<dyn std::error::Error + Send + Sync>,
}

impl std::convert::From<ActorErrorMessage> for ActorError {
    fn from(e: ActorErrorMessage) -> Self {
        Self::Other(Arc::new(e))
    }
}

impl<E: 'static> std::convert::From<E> for ActorError
where
    E: std::error::Error + Send + Sync,
{
    fn from(err: E) -> Self {
        ActorErrorMessage {
            inner: Box::new(err),
        }
        .into()
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
    pub fn downgrade(&self) -> WeakAddr<A> {
        WeakAddr {
            tx: Arc::downgrade(&self.tx),
        }
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
    pub fn upgrade(self) -> Option<Addr<A>> {
        self.tx.upgrade().map(|tx| Addr { tx })
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
                    error!("Unable to receive actor task result: {:?}", e.to_string());
                    Err(ActorError::RecvError)
                }
            },
            Err(e) => {
                error!("Unable to send actor task: {:?}", e.to_string());
                Err(ActorError::SendError)
            }
        }
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
        tokio::task::spawn(async move {
            let actor = actor;
            // TODO Sending a message to the ctx in on_start implementation will probably cause a deadlock.
            match actor.on_start(ctx).await.err() {
                None => {
                    while let Some(mut msg) = rx.recv().await {
                        msg.inner.execute(&actor).await
                    }
                    info!("Actor shutting down. All senders dropped.");
                }
                Some(e) => {
                    error!("Error during actor start: {:?}", e);
                }
            }
        });
        addr
    }

    async fn on_start(&self, _ctx: ActorContext<Self>) -> Result<(), ActorError> {
        // TODO Don't do this. Message handling has not been started.
        // Spawn a new task or something instead (it wont complete until after handling starts)
        //ctx.addr.upgrade().unwrap().send(SomeMessage).await??;

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
