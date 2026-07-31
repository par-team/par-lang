//package: basic
//! SQL database access backed by `sqlx`.
//!
//! A `Database` is a linear handle to a shared pool (`AnyPool`). `sqlx`'s `Any`
//! driver selects PostgreSQL, MySQL, or SQLite from the connection URL scheme.
//!
//! Resource model:
//! - `.split` clones the pool handle; it checks out no physical connection.
//! - `.execute` checks out a connection for one statement. It is handled inline
//!   in the database loop, so statements on one handle run in submission order.
//! - `.query` checks out a connection for the lifetime of the returned stream.
//! - `.transaction` checks out one connection for the transaction's lifetime.
//! - `.close` drops only this pool handle. The underlying pool (an `Arc` inside
//!   `sqlx`) stays alive while any sibling handle, stream, transaction, or
//!   active operation still holds a clone or a checked-out connection.
//!
//! Every external wait is bounded with a timeout. No data-dependent path
//! panics: bad SQL, bad input, driver errors, unsupported column types, and
//! invalid temporal text all become an `Error` (a human-readable string).

use std::sync::Once;
use std::time::Duration;

use arcstr::literal;
use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use num_bigint::{BigInt, BigUint};
use num_traits::ToPrimitive;
use par_runtime::external_def;
use tokio::time::timeout;

use sqlx::any::{AnyArguments, AnyPoolOptions, AnyRow};
use sqlx::{Any, AnyPool, Column, Row as _, Transaction, TypeInfo, ValueRef};

use par_runtime::primitive::{Number, ParString, Primitive};
use par_runtime::readback::{Data, Handle};

use crate::builtin::list::readback_list;

external_def! {
    @basic/Sql.Open => sql_open
}

// All external waits are bounded by these defaults. A future `OpenWith(config)`
// could expose them without changing the resource model.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
const OP_TIMEOUT: Duration = Duration::from_secs(30);
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

type AnyQuery<'q> = sqlx::query::Query<'q, Any, AnyArguments<'q>>;

static INSTALL_DRIVERS: Once = Once::new();

fn install_drivers() {
    INSTALL_DRIVERS.call_once(sqlx::any::install_default_drivers);
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

async fn sql_open(mut handle: Handle) {
    let url = handle.receive().string().await;
    match open_pool(url.as_str()).await {
        Ok(pool) => {
            handle.signal(literal!("ok"));
            provide_database(handle, pool).await;
        }
        Err(err) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(err));
        }
    }
}

async fn open_pool(url: &str) -> Result<AnyPool, String> {
    install_drivers();

    let mut options = AnyPoolOptions::new().acquire_timeout(ACQUIRE_TIMEOUT);

    // An in-memory SQLite database only lives while a connection is open, and is
    // only shared across connections in shared-cache mode. Pinning the pool to a
    // single immortal connection keeps every operation on the same database. The
    // idle/lifetime limits must be disabled: with sqlx's defaults, the pool's
    // reaper would eventually replace the connection, and the replacement would
    // see a fresh, empty database.
    options = if is_sqlite_memory(url) {
        options
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
    } else {
        options.max_connections(5)
    };

    let pool = match timeout(CONNECT_TIMEOUT, options.connect(url)).await {
        Ok(Ok(pool)) => pool,
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => return Err("connection timed out".to_string()),
    };

    // Validate connectivity with a bounded acquire, then return the connection.
    match timeout(ACQUIRE_TIMEOUT, pool.acquire()).await {
        Ok(Ok(conn)) => drop(conn),
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => return Err("connection validation timed out".to_string()),
    }

    Ok(pool)
}

fn is_sqlite_memory(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("sqlite") && (lower.contains(":memory:") || lower.contains("mode=memory"))
}

// ---------------------------------------------------------------------------
// Database protocol (iterative choice)
// ---------------------------------------------------------------------------

// Returns a boxed future so the `.split` self-recursion (this task spawning
// another `provide_database`) does not trip recursive `Send` inference.
fn provide_database(handle: Handle, pool: AnyPool) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        let mut handle = handle;
        loop {
            match handle.case().await.as_str() {
                // .split => (self) self
                "split" => {
                    let sibling = pool.clone();
                    handle
                        .send()
                        .concurrently(move |h| provide_database(h, sibling));
                }

                // .execute(String, List<Value>) => (Try<Error, Nat>) self
                //
                // Handled inline, unlike `.query` and `.transaction` (which hand off
                // long-lived leases and must not block this loop), so statements
                // issued on one handle run in submission order even when their
                // results are never inspected. Use `.split` for concurrency.
                "execute" => {
                    let sql = handle.receive().string().await.as_str().to_string();
                    let params = read_params(handle.receive()).await;
                    db_execute(handle.send(), &pool, sql, params).await;
                }

                // .query(String, List<Value>) => (Try<Error, Stream<Error, Row>>) self
                "query" => {
                    let sql = handle.receive().string().await.as_str().to_string();
                    let params = read_params(handle.receive()).await;
                    let pool = pool.clone();
                    handle
                        .send()
                        .concurrently(move |h| db_query(h, pool, sql, params));
                }

                // .transaction => (Try<Error, Transaction>) self
                "transaction" => {
                    let pool = pool.clone();
                    handle.send().concurrently(move |h| db_transaction(h, pool));
                }

                // .close => !
                "close" | "#close" => {
                    handle.break_();
                    drop(pool);
                    return;
                }

                _ => unreachable!(),
            }
        }
    })
}

async fn db_execute(mut handle: Handle, pool: &AnyPool, sql: String, params: Vec<Data>) {
    match run_execute(pool, &sql, &params).await {
        Ok(affected) => {
            handle.signal(literal!("ok"));
            handle.provide_nat(BigUint::from(affected));
        }
        Err(err) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(err));
        }
    }
}

async fn run_execute(pool: &AnyPool, sql: &str, params: &[Data]) -> Result<u64, String> {
    let query = build_query(sql, params)?;
    let result = match timeout(OP_TIMEOUT, query.execute(pool)).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => return Err("statement timed out".to_string()),
    };
    Ok(result.rows_affected())
}

async fn db_query(mut handle: Handle, pool: AnyPool, sql: String, params: Vec<Data>) {
    // Bounded acquire. The stream owns this connection until it is fully drained
    // or cancelled, at which point the connection returns to the pool.
    let mut conn = match timeout(ACQUIRE_TIMEOUT, pool.acquire()).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(err)) => return query_startup_err(handle, err.to_string()),
        Err(_) => return query_startup_err(handle, "connection acquire timed out".to_string()),
    };

    let query = match build_query(&sql, &params) {
        Ok(query) => query,
        Err(err) => return query_startup_err(handle, err),
    };

    let mut stream = query.fetch(&mut *conn);

    // Fetch the first row eagerly so acquisition/startup failures land in the
    // outer `Try`, while mid-result failures surface at the stream's `.end`.
    let first = match timeout(STEP_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(row))) => match row_to_cells(&row) {
            Ok(cells) => Some(cells),
            Err(err) => return query_startup_err(handle, err),
        },
        Ok(Some(Err(err))) => return query_startup_err(handle, err.to_string()),
        Ok(None) => None,
        Err(_) => return query_startup_err(handle, "query start timed out".to_string()),
    };

    // Startup succeeded: hand back `.ok stream`.
    handle.signal(literal!("ok"));

    let mut pending = first;
    loop {
        let next = match pending.take() {
            Some(cells) => Some(Ok(cells)),
            None => match timeout(STEP_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(row))) => Some(row_to_cells(&row)),
                Ok(Some(Err(err))) => Some(Err(err.to_string())),
                Ok(None) => None,
                Err(_) => Some(Err("query step timed out".to_string())),
            },
        };

        match next {
            // Stream exhausted: .end .ok!
            None => {
                handle.signal(literal!("end"));
                handle.signal(literal!("ok"));
                return handle.break_();
            }
            // Mid-result failure: .end .err
            Some(Err(err)) => {
                handle.signal(literal!("end"));
                handle.signal(literal!("err"));
                return handle.provide_string(ParString::from(err));
            }
            // One row available: .item choice { .cancel, .get }
            Some(Ok(cells)) => {
                handle.signal(literal!("item"));
                match handle.case().await.as_str() {
                    "cancel" | "#close" => {
                        handle.signal(literal!("ok"));
                        return handle.break_();
                    }
                    "get" => {
                        provide_row(handle.send(), &cells);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

fn query_startup_err(mut handle: Handle, message: String) {
    handle.signal(literal!("err"));
    handle.provide_string(ParString::from(message));
}

async fn db_transaction(mut handle: Handle, pool: AnyPool) {
    match timeout(ACQUIRE_TIMEOUT, pool.begin()).await {
        Ok(Ok(tx)) => {
            handle.signal(literal!("ok"));
            provide_transaction(handle, tx).await;
        }
        Ok(Err(err)) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(err.to_string()));
        }
        Err(_) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from("transaction begin timed out"));
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction protocol (iterative choice)
// ---------------------------------------------------------------------------

async fn provide_transaction(mut handle: Handle, mut tx: Transaction<'static, Any>) {
    loop {
        match handle.case().await.as_str() {
            // .execute(String, List<Value>) => Try<Error, (Nat) self>
            "execute" => {
                let sql = handle.receive().string().await.as_str().to_string();
                let params = read_params(handle.receive()).await;
                match tx_execute(&mut tx, &sql, &params).await {
                    Ok(affected) => {
                        handle.signal(literal!("ok"));
                        handle.send().provide_nat(BigUint::from(affected));
                    }
                    Err(err) => {
                        // No `self`: drop `tx` (rolling back) and report the error.
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from(err));
                        return;
                    }
                }
            }

            // .query(String, List<Value>) => Try<Error, (List<Row>) self>
            "query" => {
                let sql = handle.receive().string().await.as_str().to_string();
                let params = read_params(handle.receive()).await;
                match tx_query(&mut tx, &sql, &params).await {
                    Ok(rows) => {
                        handle.signal(literal!("ok"));
                        provide_rows(handle.send(), &rows);
                    }
                    Err(err) => {
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from(err));
                        return;
                    }
                }
            }

            // .commit => Try<Error, !>
            "commit" => {
                match timeout(OP_TIMEOUT, tx.commit()).await {
                    Ok(Ok(())) => {
                        handle.signal(literal!("ok"));
                        handle.break_();
                    }
                    Ok(Err(err)) => {
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from(err.to_string()));
                    }
                    Err(_) => {
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from("commit timed out"));
                    }
                }
                return;
            }

            // .rollback* => Try<Error, !>
            "rollback" | "#close" => {
                match timeout(OP_TIMEOUT, tx.rollback()).await {
                    Ok(Ok(())) => {
                        handle.signal(literal!("ok"));
                        handle.break_();
                    }
                    Ok(Err(err)) => {
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from(err.to_string()));
                    }
                    Err(_) => {
                        handle.signal(literal!("err"));
                        handle.provide_string(ParString::from("rollback timed out"));
                    }
                }
                return;
            }

            _ => unreachable!(),
        }
    }
}

async fn tx_execute(
    tx: &mut Transaction<'static, Any>,
    sql: &str,
    params: &[Data],
) -> Result<u64, String> {
    let query = build_query(sql, params)?;
    let result = match timeout(OP_TIMEOUT, query.execute(&mut **tx)).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => return Err("statement timed out".to_string()),
    };
    Ok(result.rows_affected())
}

async fn tx_query(
    tx: &mut Transaction<'static, Any>,
    sql: &str,
    params: &[Data],
) -> Result<Vec<Vec<(String, Data)>>, String> {
    let query = build_query(sql, params)?;
    let rows = match timeout(OP_TIMEOUT, query.fetch_all(&mut **tx)).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(err)) => return Err(err.to_string()),
        Err(_) => return Err("query timed out".to_string()),
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_cells(row)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parameters and rows
// ---------------------------------------------------------------------------

async fn read_params(handle: Handle) -> Vec<Data> {
    readback_list(handle, |item| async move { item.data().await }).await
}

fn build_query<'q>(sql: &'q str, params: &[Data]) -> Result<AnyQuery<'q>, String> {
    let mut query = sqlx::query(sql);
    for param in params {
        query = bind_param(query, param)?;
    }
    Ok(query)
}

fn bind_param<'q>(query: AnyQuery<'q>, value: &Data) -> Result<AnyQuery<'q>, String> {
    let Data::Either(tag, payload) = value else {
        return Err("invalid SQL parameter".to_string());
    };
    match tag.as_str() {
        // Untyped NULL. Works whenever the SQL context gives the backend enough
        // type information; otherwise the caller must cast in SQL.
        "null" => Ok(query.bind(Option::<String>::None)),
        "bool" => Ok(query.bind(data_bool(payload)?)),
        "int" => {
            let big = data_int(payload)?;
            let narrow = big
                .to_i64()
                .ok_or_else(|| "integer parameter out of range for 64-bit binding".to_string())?;
            Ok(query.bind(narrow))
        }
        "float" => {
            let value = data_float(payload)?;
            if !value.is_finite() {
                return Err("cannot bind a non-finite floating-point parameter".to_string());
            }
            Ok(query.bind(value))
        }
        "text" => Ok(query.bind(data_string(payload)?)),
        "bytes" => Ok(query.bind(data_bytes(payload)?)),
        other => Err(format!("unsupported SQL parameter variant: .{other}")),
    }
}

fn data_bool(data: &Data) -> Result<bool, String> {
    match data {
        Data::Either(tag, _) if tag.as_str() == "true" => Ok(true),
        Data::Either(tag, _) if tag.as_str() == "false" => Ok(false),
        _ => Err("invalid boolean parameter".to_string()),
    }
}

fn data_int(data: &Data) -> Result<BigInt, String> {
    match data {
        Data::Primitive(Primitive::Number(Number::Zero)) => Ok(BigInt::ZERO),
        Data::Primitive(Primitive::Number(Number::Int(value))) => Ok(value.clone()),
        _ => Err("invalid integer parameter".to_string()),
    }
}

fn data_float(data: &Data) -> Result<f64, String> {
    match data {
        Data::Primitive(Primitive::Number(Number::Zero)) => Ok(0.0),
        Data::Primitive(Primitive::Number(Number::Float(value))) => Ok(*value),
        Data::Primitive(Primitive::Number(Number::Int(value))) => value
            .to_f64()
            .ok_or_else(|| "invalid floating-point parameter".to_string()),
        _ => Err("invalid floating-point parameter".to_string()),
    }
}

fn data_string(data: &Data) -> Result<String, String> {
    match data {
        Data::Primitive(Primitive::String(value)) => Ok(value.as_str().to_string()),
        _ => Err("invalid text parameter".to_string()),
    }
}

fn data_bytes(data: &Data) -> Result<Vec<u8>, String> {
    match data {
        Data::Primitive(Primitive::Bytes(value)) => Ok(value.to_vec()),
        _ => Err("invalid byte parameter".to_string()),
    }
}

// Provide a `Row` (`List<(String) Value>`).
fn provide_row(mut handle: Handle, cells: &[(String, Data)]) {
    for (name, value) in cells {
        handle.signal(literal!("item"));
        let mut pair = handle.send();
        pair.send().provide_string(ParString::from(name.clone()));
        pair.provide_data(value);
    }
    handle.signal(literal!("end"));
    handle.break_();
}

// Provide a `List<Row>`.
fn provide_rows(mut handle: Handle, rows: &[Vec<(String, Data)>]) {
    for cells in rows {
        handle.signal(literal!("item"));
        provide_row(handle.send(), cells);
    }
    handle.signal(literal!("end"));
    handle.break_();
}

fn row_to_cells(row: &AnyRow) -> Result<Vec<(String, Data)>, String> {
    let mut cells = Vec::with_capacity(row.len());
    for column in row.columns() {
        let name = column.name().to_string();
        let value = cell_value(row, column.ordinal())?;
        cells.push((name, value));
    }
    Ok(cells)
}

// Classify a column by its concrete value. `Any` decodes strictly by storage
// kind (no lossy coercion), so trying the supported types in order yields the
// faithful `Value` variant. Unsupported column types error instead of silently
// becoming text.
fn cell_value(row: &AnyRow, index: usize) -> Result<Data, String> {
    let raw = row.try_get_raw(index).map_err(|err| err.to_string())?;
    if raw.is_null() {
        return Ok(value_null());
    }
    if let Ok(value) = row.try_get::<bool, _>(index) {
        return Ok(value_bool(value));
    }
    if let Ok(value) = row.try_get::<i64, _>(index) {
        return Ok(value_int(value));
    }
    if let Ok(value) = row.try_get::<f64, _>(index) {
        return Ok(value_float(value));
    }
    if let Ok(value) = row.try_get::<String, _>(index) {
        return Ok(value_text(&value));
    }
    if let Ok(value) = row.try_get::<Vec<u8>, _>(index) {
        return Ok(value_bytes(&value));
    }
    Err(format!(
        "unsupported column type: {}",
        raw.type_info().name()
    ))
}

// ---------------------------------------------------------------------------
// `Value` constructors (as runtime `Data` matching the Par `Value` either)
// ---------------------------------------------------------------------------

fn value_null() -> Data {
    Data::Either(literal!("null"), Box::new(Data::Unit))
}

fn value_bool(value: bool) -> Data {
    let variant = if value {
        literal!("true")
    } else {
        literal!("false")
    };
    Data::Either(
        literal!("bool"),
        Box::new(Data::Either(variant, Box::new(Data::Unit))),
    )
}

fn value_int(value: i64) -> Data {
    Data::Either(
        literal!("int"),
        Box::new(Data::Primitive(Primitive::Number(Number::Int(
            BigInt::from(value),
        )))),
    )
}

fn value_float(value: f64) -> Data {
    Data::Either(
        literal!("float"),
        Box::new(Data::Primitive(Primitive::Number(Number::Float(value)))),
    )
}

fn value_text(value: &str) -> Data {
    Data::Either(
        literal!("text"),
        Box::new(Data::Primitive(Primitive::String(
            ParString::copy_from_slice(value),
        ))),
    )
}

fn value_bytes(value: &[u8]) -> Data {
    Data::Either(
        literal!("bytes"),
        Box::new(Data::Primitive(Primitive::Bytes(Bytes::copy_from_slice(
            value,
        )))),
    )
}
