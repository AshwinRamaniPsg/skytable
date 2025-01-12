/*
 * Created on Sun Apr 25 2021
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2020, Sayan Nandan <ohsayan@outlook.com>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 *
*/

//! # Generic connection traits
//! The `con` module defines the generic connection traits `ProtocolConnection` and `ProtocolConnectionExt`.
//! These two traits can be used to interface with sockets that are used for communication through the Skyhash
//! protocol.
//!
//! The `ProtocolConnection` trait provides a basic set of methods that are required by prospective connection
//! objects to be eligible for higher level protocol interactions (such as interactions with high-level query objects).
//! Once a type implements this trait, it automatically gets a free `ProtocolConnectionExt` implementation. This immediately
//! enables this connection object/type to use methods like read_query enabling it to read and interact with queries and write
//! respones in compliance with the Skyhash protocol.

use crate::{
    actions::{ActionError, ActionResult},
    auth::{self, AuthProvider},
    corestore::{buffers::Integer64, Corestore},
    dbnet::{
        connection::prelude::FutureResult,
        tcp::{BufferedSocketStream, Connection},
        Terminator,
    },
    protocol::{self, responses, ParseError, Query},
    queryengine,
    resp::Writable,
    IoResult,
};
use bytes::{Buf, BytesMut};
use std::{
    future::Future,
    io::{Error as IoError, ErrorKind},
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    sync::{mpsc, Semaphore},
};

pub const SIMPLE_QUERY_HEADER: [u8; 1] = [b'*'];
type QueryWithAdvance = (Query, usize);

pub enum QueryResult {
    Q(QueryWithAdvance),
    E(&'static [u8]),
    Wrongtype,
    Disconnected,
}

pub struct AuthProviderHandle<'a, T, Strm> {
    provider: &'a mut AuthProvider,
    executor: &'a mut ExecutorFn<T, Strm>,
    _phantom: PhantomData<(T, Strm)>,
}

impl<'a, T, Strm> AuthProviderHandle<'a, T, Strm>
where
    T: ClientConnection<Strm>,
    Strm: Stream,
{
    pub fn new(provider: &'a mut AuthProvider, executor: &'a mut ExecutorFn<T, Strm>) -> Self {
        Self {
            provider,
            executor,
            _phantom: PhantomData,
        }
    }
    pub fn provider_mut(&mut self) -> &mut AuthProvider {
        self.provider
    }
    pub fn provider(&self) -> &AuthProvider {
        self.provider
    }
    pub fn swap_executor_to_anonymous(&mut self) {
        *self.executor = ConnectionHandler::execute_unauth;
    }
    pub fn swap_executor_to_authenticated(&mut self) {
        *self.executor = ConnectionHandler::execute_auth;
    }
}

pub mod prelude {
    //! A 'prelude' for callers that would like to use the `ProtocolConnection` and `ProtocolConnectionExt` traits
    //!
    //! This module is hollow itself, it only re-exports from `dbnet::con` and `tokio::io`
    pub use super::{AuthProviderHandle, ClientConnection, ProtocolConnectionExt, Stream};
    pub use crate::actions::{ensure_boolean_or_aerr, ensure_cond_or_err, ensure_length};
    pub use crate::corestore::{
        table::{KVEBlob, KVEList},
        Corestore,
    };
    pub use crate::protocol::responses::{self, groups};
    pub use crate::queryengine::ActionIter;
    pub use crate::resp::StringWrapper;
    pub use crate::util::{self, FutureResult, UnwrapActionError, Unwrappable};
    pub use crate::{aerr, conwrite, get_tbl, handle_entity, is_lowbit_set, registry};
    pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
}

/// # The `ProtocolConnectionExt` trait
///
/// The `ProtocolConnectionExt` trait has default implementations and doesn't ever require explicit definitions, unless
/// there's some black magic that you want to do. All [`ProtocolConnection`] objects will get a free implementation for this trait.
/// Hence implementing [`ProtocolConnection`] alone is enough for you to get high-level methods to interface with the protocol.
///
/// ## DO NOT
/// The fact that this is a trait enables great flexibility in terms of visibility, but **DO NOT EVER CALL any function other than
/// `read_query`, `close_conn_with_error` or `write_response`**. If you mess with functions like `read_again`, you're likely to pull yourself into some
/// good trouble.
pub trait ProtocolConnectionExt<Strm>: ProtocolConnection<Strm> + Send
where
    Strm: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync,
{
    /// Try to parse a query from the buffered data
    fn try_query(&self) -> Result<QueryWithAdvance, ParseError> {
        protocol::Parser::parse(self.get_buffer())
    }
    /// Read a query from the remote end
    ///
    /// This function asynchronously waits until all the data required
    /// for parsing the query is available
    fn read_query<'r, 's>(
        &'r mut self,
    ) -> Pin<Box<dyn Future<Output = Result<QueryResult, IoError>> + Send + 's>>
    where
        'r: 's,
        Self: Sync + Send + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            loop {
                let (buffer, stream) = mv_self.get_mut_both();
                match stream.read_buf(buffer).await {
                    Ok(0) => {
                        if buffer.is_empty() {
                            return Ok(QueryResult::Disconnected);
                        } else {
                            return Err(IoError::from(ErrorKind::ConnectionReset));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => return Err(e),
                }
                match mv_self.try_query() {
                    Ok(query_with_advance) => {
                        return Ok(QueryResult::Q(query_with_advance));
                    }
                    Err(ParseError::NotEnough) => (),
                    Err(ParseError::DatatypeParseFailure) => return Ok(QueryResult::Wrongtype),
                    Err(ParseError::UnexpectedByte) | Err(ParseError::BadPacket) => {
                        return Ok(QueryResult::E(responses::full_responses::R_PACKET_ERR));
                    }
                }
            }
        })
    }
    /// Write a response to the stream
    fn write_response<'r, 's>(
        &'r mut self,
        streamer: impl Writable + 's + Send + Sync,
    ) -> Pin<Box<dyn Future<Output = IoResult<()>> + Sync + Send + 's>>
    where
        'r: 's,
        Self: Send + 's,
        Self: Sync,
    {
        Box::pin(async move {
            let mv_self = self;
            let streamer = streamer;
            let ret: IoResult<()> = {
                streamer.write(&mut mv_self.get_mut_stream()).await?;
                Ok(())
            };
            ret
        })
    }
    /// Write the simple query header `*1\n` to the stream
    fn write_simple_query_header<'r, 's>(
        &'r mut self,
    ) -> Pin<Box<dyn Future<Output = IoResult<()>> + Send + Sync + 's>>
    where
        'r: 's,
        Self: Send + Sync + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            let ret: IoResult<()> = {
                mv_self.write_response(SIMPLE_QUERY_HEADER).await?;
                Ok(())
            };
            ret
        })
    }
    /// Write the length of the pipeline query (*)
    fn write_pipeline_query_header<'r, 's>(
        &'r mut self,
        len: usize,
    ) -> FutureResult<'s, IoResult<()>>
    where
        'r: 's,
        Self: Send + Sync + 's,
    {
        Box::pin(async move {
            let slf = self;
            slf.write_response([b'$']).await?;
            slf.get_mut_stream()
                .write_all(&Integer64::init(len as u64))
                .await?;
            slf.write_response([b'\n']).await?;
            Ok(())
        })
    }
    /// Write the flat array length (`_<size>\n`)
    fn write_flat_array_length<'r, 's>(&'r mut self, len: usize) -> FutureResult<'s, IoResult<()>>
    where
        'r: 's,
        Self: Send + Sync + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            let ret: IoResult<()> = {
                mv_self.write_response([b'_']).await?;
                mv_self.write_response(len.to_string().into_bytes()).await?;
                mv_self.write_response([b'\n']).await?;
                Ok(())
            };
            ret
        })
    }
    /// Write the array length (`&<size>\n`)
    fn write_array_length<'r, 's>(&'r mut self, len: usize) -> FutureResult<'s, IoResult<()>>
    where
        'r: 's,
        Self: Send + Sync + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            let ret: IoResult<()> = {
                mv_self.write_response([b'&']).await?;
                mv_self.write_response(len.to_string().into_bytes()).await?;
                mv_self.write_response([b'\n']).await?;
                Ok(())
            };
            ret
        })
    }
    /// Wraps around the `write_response` used to differentiate between a
    /// success response and an error response
    fn close_conn_with_error<'r, 's>(
        &'r mut self,
        resp: impl Writable + 's + Send + Sync,
    ) -> FutureResult<'s, IoResult<()>>
    where
        'r: 's,
        Self: Send + Sync + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            let ret: IoResult<()> = {
                mv_self.write_response(resp).await?;
                mv_self.flush_stream().await?;
                Ok(())
            };
            ret
        })
    }
    fn flush_stream<'r, 's>(&'r mut self) -> FutureResult<'s, IoResult<()>>
    where
        'r: 's,
        Self: Sync + Send + 's,
    {
        Box::pin(async move {
            let mv_self = self;
            let ret: IoResult<()> = {
                mv_self.get_mut_stream().flush().await?;
                Ok(())
            };
            ret
        })
    }
    unsafe fn raw_stream(&mut self) -> &mut BufWriter<Strm> {
        self.get_mut_stream()
    }
}

/// # The `ProtocolConnection` trait
///
/// The `ProtocolConnection` trait has low-level methods that can be used to interface with raw sockets. Any type
/// that successfully implements this trait will get an implementation for `ProtocolConnectionExt` which augments and
/// builds on these fundamental methods to provide high-level interfacing with queries.
///
/// ## Example of a `ProtocolConnection` object
/// Ideally a `ProtocolConnection` object should look like (the generic parameter just exists for doc-tests, just think that
/// there is a type `Strm`):
/// ```no_run
/// struct Connection<Strm> {
///     pub buffer: bytes::BytesMut,
///     pub stream: Strm,
/// }
/// ```
///
/// `Strm` should be a stream, i.e something like an SSL connection/TCP connection.
pub trait ProtocolConnection<Strm> {
    /// Returns an **immutable** reference to the underlying read buffer
    fn get_buffer(&self) -> &BytesMut;
    /// Returns an **immutable** reference to the underlying stream
    fn get_stream(&self) -> &BufWriter<Strm>;
    /// Returns a **mutable** reference to the underlying read buffer
    fn get_mut_buffer(&mut self) -> &mut BytesMut;
    /// Returns a **mutable** reference to the underlying stream
    fn get_mut_stream(&mut self) -> &mut BufWriter<Strm>;
    /// Returns a **mutable** reference to (buffer, stream)
    ///
    /// This is to avoid double mutable reference errors
    fn get_mut_both(&mut self) -> (&mut BytesMut, &mut BufWriter<Strm>);
    /// Advance the read buffer by `forward_by` positions
    fn advance_buffer(&mut self, forward_by: usize) {
        self.get_mut_buffer().advance(forward_by)
    }
    /// Clear the internal buffer completely
    fn clear_buffer(&mut self) {
        self.get_mut_buffer().clear()
    }
}

// Give ProtocolConnection implementors a free ProtocolConnectionExt impl

impl<Strm, T> ProtocolConnectionExt<Strm> for T
where
    T: ProtocolConnection<Strm> + Send,
    Strm: Sync + Send + Unpin + AsyncWriteExt + AsyncReadExt,
{
}

impl<T> ProtocolConnection<T> for Connection<T>
where
    T: BufferedSocketStream,
{
    fn get_buffer(&self) -> &BytesMut {
        &self.buffer
    }
    fn get_stream(&self) -> &BufWriter<T> {
        &self.stream
    }
    fn get_mut_buffer(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }
    fn get_mut_stream(&mut self) -> &mut BufWriter<T> {
        &mut self.stream
    }
    fn get_mut_both(&mut self) -> (&mut BytesMut, &mut BufWriter<T>) {
        (&mut self.buffer, &mut self.stream)
    }
}

pub(super) type ExecutorFn<T, Strm> =
    for<'s> fn(&'s mut ConnectionHandler<T, Strm>, Query) -> FutureResult<'s, ActionResult<()>>;

/// # A generic connection handler
///
/// A [`ConnectionHandler`] object is a generic connection handler for any object that implements the [`ProtocolConnection`] trait (or
/// the [`ProtocolConnectionExt`] trait). This function will accept such a type `T`, possibly a listener object and then use it to read
/// a query, parse it and return an appropriate response through [`corestore::Corestore::execute_query`]
pub struct ConnectionHandler<T, Strm> {
    db: Corestore,
    con: T,
    climit: Arc<Semaphore>,
    auth: AuthProvider,
    executor: ExecutorFn<T, Strm>,
    terminator: Terminator,
    _term_sig_tx: mpsc::Sender<()>,
    _marker: PhantomData<Strm>,
}

impl<T, Strm> ConnectionHandler<T, Strm>
where
    T: ProtocolConnectionExt<Strm> + Send + Sync,
    Strm: Sync + Send + Unpin + AsyncWriteExt + AsyncReadExt,
{
    pub fn new(
        db: Corestore,
        con: T,
        auth: AuthProvider,
        executor: ExecutorFn<T, Strm>,
        climit: Arc<Semaphore>,
        terminator: Terminator,
        _term_sig_tx: mpsc::Sender<()>,
    ) -> Self {
        Self {
            db,
            con,
            auth,
            climit,
            executor,
            terminator,
            _term_sig_tx,
            _marker: PhantomData,
        }
    }
    pub async fn run(&mut self) -> IoResult<()> {
        while !self.terminator.is_termination_signal() {
            let try_df = tokio::select! {
                tdf = self.con.read_query() => tdf,
                _ = self.terminator.receive_signal() => {
                    return Ok(());
                }
            };
            match try_df {
                Ok(QueryResult::Q((query, advance_by))) => {
                    // the mutable reference to self ensures that the buffer is not modified
                    // hence ensuring that the pointers will remain valid
                    match self.execute_query(query).await {
                        Ok(()) => {}
                        Err(ActionError::ActionError(e)) => {
                            self.con.close_conn_with_error(e).await?;
                        }
                        Err(ActionError::IoError(e)) => {
                            return Err(e);
                        }
                    }
                    // this is only when we clear the buffer. since execute_query is not called
                    // at this point, it's totally fine (so invalidating ptrs is totally cool)
                    self.con.advance_buffer(advance_by);
                }
                Ok(QueryResult::E(r)) => self.con.close_conn_with_error(r).await?,
                Ok(QueryResult::Wrongtype) => {
                    self.con
                        .close_conn_with_error(responses::groups::WRONGTYPE_ERR.to_owned())
                        .await?
                }
                Ok(QueryResult::Disconnected) => return Ok(()),
                #[cfg(windows)]
                Err(e) => match e.kind() {
                    ErrorKind::ConnectionReset => return Ok(()),
                    _ => return Err(e),
                },
                #[cfg(not(windows))]
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Execute queries for an unauthenticated user
    pub(super) fn execute_unauth(&mut self, query: Query) -> FutureResult<'_, ActionResult<()>> {
        Box::pin(async move {
            let con = &mut self.con;
            let db = &mut self.db;
            let mut auth_provider = AuthProviderHandle::new(&mut self.auth, &mut self.executor);
            match query {
                Query::Simple(sq) => {
                    con.write_simple_query_header().await?;
                    queryengine::execute_simple_noauth(db, con, &mut auth_provider, sq).await?;
                }
                Query::Pipelined(_) => {
                    con.write_simple_query_header().await?;
                    con.write_response(auth::errors::AUTH_CODE_BAD_CREDENTIALS)
                        .await?;
                }
            }
            Ok(())
        })
    }

    /// Execute queries for an authenticated user
    pub(super) fn execute_auth(&mut self, query: Query) -> FutureResult<'_, ActionResult<()>> {
        Box::pin(async move {
            let con = &mut self.con;
            let db = &mut self.db;
            let mut auth_provider = AuthProviderHandle::new(&mut self.auth, &mut self.executor);
            match query {
                Query::Simple(q) => {
                    con.write_simple_query_header().await?;
                    queryengine::execute_simple(db, con, &mut auth_provider, q).await?;
                }
                Query::Pipelined(pipeline) => {
                    con.write_pipeline_query_header(pipeline.len()).await?;
                    queryengine::execute_pipeline(db, con, &mut auth_provider, pipeline).await?;
                }
            }
            Ok(())
        })
    }

    /// Execute a query that has already been validated by `Connection::read_query`
    async fn execute_query(&mut self, query: Query) -> ActionResult<()> {
        (self.executor)(self, query).await?;
        self.con.flush_stream().await?;
        Ok(())
    }
}

impl<T, Strm> Drop for ConnectionHandler<T, Strm> {
    fn drop(&mut self) {
        // Make sure that the permit is returned to the semaphore
        // in the case that there is a panic inside
        self.climit.add_permits(1);
    }
}

/// A simple _shorthand trait_ for the insanely long definition of the TCP-based stream generic type
pub trait Stream: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync {}
impl<T> Stream for T where T: AsyncReadExt + AsyncWriteExt + Unpin + Send + Sync {}

/// A simple _shorthand trait_ for the insanely long definition of the connection generic type
pub trait ClientConnection<Strm: Stream>: ProtocolConnectionExt<Strm> + Send + Sync {}
impl<T, Strm> ClientConnection<Strm> for T
where
    T: ProtocolConnectionExt<Strm> + Send + Sync,
    Strm: Stream,
{
}
