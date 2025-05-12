#[cfg(feature = "actix")]
pub mod actix;

use dashmap::{DashMap, Entry, mapref::one::Ref};
use serde::{Deserialize, Serialize};
use std::{
    process::{ExitStatus, Stdio},
    sync::Arc,
};
use tokio::{
    process::{ChildStderr, ChildStdin, ChildStdout, Command},
    sync::{Mutex, OwnedMutexGuard},
};
use uuid::Uuid;

#[derive(Default)]
pub struct Kindergarten {
    access: DashMap<Ticket, Kind>,
}

#[derive(Debug, Clone)]
pub struct Kind {
    inner: KindInner,
}

#[derive(Debug, Clone)]
struct KindInner {
    inner: Arc<Mutex<tokio::process::Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<ChildStdout>>,
    stderr: Arc<Mutex<ChildStderr>>,
}

impl Kind {
    fn new(mut cmd: Command) -> std::io::Result<Self> {
        let mut inner = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = inner.stdin.take().unwrap();
        let stdout = inner.stdout.take().unwrap();
        let stderr = inner.stderr.take().unwrap();

        Ok(Self {
            inner: KindInner {
                inner: Arc::new(Mutex::new(inner)),
                stdin: Arc::new(Mutex::new(stdin)),
                stdout: Arc::new(Mutex::new(stdout)),
                stderr: Arc::new(Mutex::new(stderr)),
            },
        })
    }

    pub async fn stdin(&self) -> OwnedMutexGuard<ChildStdin> {
        self.inner.stdin.clone().lock_owned().await
    }

    pub async fn stdout(&self) -> OwnedMutexGuard<ChildStdout> {
        self.inner.stdout.clone().lock_owned().await
    }

    pub async fn stderr(&self) -> OwnedMutexGuard<ChildStderr> {
        self.inner.stderr.clone().lock_owned().await
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Ticket(Uuid);

impl<'de> Deserialize<'de> for Ticket {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Uuid::deserialize(deserializer).map(Ticket)
    }
}

impl Serialize for Ticket {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl Kindergarten {
    pub async fn spawn(&self, cmd: Command) -> Result<Ticket, std::io::Error> {
        let ticket = Ticket(Uuid::new_v4());
        self.access.insert(ticket, Kind::new(cmd)?);
        Ok(ticket)
    }

    /// Gets a handle to a child instance, streams must be locked separately to use
    ///
    /// Directly using the getters for streams is slightly more efficient
    pub fn get(&self, t: Ticket) -> Option<Kind> {
        self.access.get(&t).map(|k| k.value().clone())
    }

    /// Gets a handle to a child instance, streams must be locked separately to use
    ///
    /// Directly using the getters for streams is slightly more efficient
    pub fn get_or_insert_with(&self, t: Ticket, f: impl Fn() -> Command) -> std::io::Result<Kind> {
        Ok(match self.access.entry(t) {
            Entry::Occupied(occupied_entry) => occupied_entry.get().clone(),
            Entry::Vacant(vacant_entry) => vacant_entry.insert(Kind::new((f)())?).value().clone(),
        })
    }

    pub async fn stdin(&self, t: Ticket) -> Option<OwnedMutexGuard<ChildStdin>> {
        match self.access.get(&t) {
            Some(child) => Some(child.inner.stdin.clone().lock_owned().await),
            None => None,
        }
    }

    pub async fn stdout(&self, t: Ticket) -> Option<OwnedMutexGuard<ChildStdout>> {
        match self.access.get(&t) {
            Some(child) => Some(child.inner.stdout.clone().lock_owned().await),
            None => None,
        }
    }

    pub async fn stderr(&self, t: Ticket) -> Option<OwnedMutexGuard<ChildStderr>> {
        match self.access.get(&t) {
            Some(child) => Some(child.inner.stderr.clone().lock_owned().await),
            None => None,
        }
    }

    pub async fn has_terminated(&self, t: Ticket) -> Option<bool> {
        match self.access.get(&t) {
            Some(x) => Some(
                x.inner
                    .inner
                    .lock()
                    .await
                    .try_wait()
                    .map(|x| x.is_some())
                    .unwrap_or_default(),
            ),
            None => None,
        }
    }

    pub async fn success(&self, t: Ticket) -> Option<std::io::Result<bool>> {
        match self.access.get(&t) {
            Some(x) => Some(
                x.inner
                    .inner
                    .lock()
                    .await
                    .try_wait()
                    .map(|x| x.is_some_and(|s| s.success())),
            ),
            None => None,
        }
    }

    pub async fn terminate(&self, t: Ticket) -> Option<std::io::Result<ExitStatus>> {
        match self.access.get(&t) {
            Some(c) => Some(Self::_terminate(c).await),
            None => None,
        }
    }

    async fn _terminate(c: Ref<'_, Ticket, Kind>) -> std::io::Result<ExitStatus> {
        let mut m = c.inner.inner.lock().await;
        m.start_kill()?;
        m.wait().await
    }

    pub async fn wait(&self, t: Ticket) -> Option<std::io::Result<ExitStatus>> {
        match self.access.get(&t) {
            Some(c) => Some(c.inner.inner.lock().await.wait().await),
            None => None,
        }
    }
}
