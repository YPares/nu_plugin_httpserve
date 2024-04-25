#![allow(warnings)]

use std::borrow::Borrow;
use std::error::Error;
use std::path::Path;

use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};

use nu_protocol::engine::Closure;
use nu_protocol::{
    LabeledError, PipelineData, RawStream, Record, ShellError, Signature, Span, Spanned,
    SyntaxShape, Type, Value,
};

// use crate::traits;
use crate::HTTPPlugin;

pub struct HTTPServe;

impl PluginCommand for HTTPServe {
    type Plugin = HTTPPlugin;

    fn name(&self) -> &str {
        "h. serve"
    }

    fn usage(&self) -> &str {
        "Service HTTP requests"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            // .required("url", SyntaxShape::String, "the url to service")
            .required(
                "closure",
                SyntaxShape::Closure(Some(vec![SyntaxShape::Record(vec![])])),
                "The closure to evaluate for each connection",
            )
            .input_output_type(Type::Any, Type::Any)
    }

    fn run(
        &self,
        plugin: &HTTPPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let (tx, mut rx) = watch::channel(false);

        match input {
            PipelineData::ExternalStream { exit_code, .. } => {
                let exit_code = exit_code.unwrap();
                std::thread::spawn(move || {
                    exit_code.stream.for_each(drop);
                    let _ = tx.send(true);
                    eprintln!("i'm outie");
                });
            }
            _ => return Err(LabeledError::new("ExternalStream expected")),
        }

        plugin.runtime.block_on(async move {
            let _ = serve(engine, call, rx).await;
        });

        let span = call.head;
        let value = Value::string("peace", span);
        let body = PipelineData::Value(value, None);
        return Ok(body);
    }
}

use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::body::{Bytes, Frame};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use tokio_stream::StreamExt;

use tokio::sync::watch;
use tokio_stream::wrappers::ReceiverStream;

use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::StreamBody;

fn run_eval(
    engine: &EngineInterface,
    call: &EvaluatedCall,
    meta: Record,
    mut rx: mpsc::Receiver<Result<Vec<u8>, hyper::Error>>,
    mut tx: mpsc::Sender<Result<Vec<u8>, hyper::Error>>,
) {
    let closure = call.req(0).unwrap();
    let span = call.head;

    let iter = std::iter::from_fn(move || {
        Some(
            rx.blocking_recv()?
                .map_err(|err| {
                    ShellError::LabeledError(Box::new(LabeledError::new(format!(
                        "Read error: {}",
                        err
                    ))))
                })
                .map(|bytes| bytes.to_vec()),
        )
    });

    let stream = RawStream::new(
        Box::new(iter) as Box<dyn Iterator<Item = Result<Vec<u8>, ShellError>> + Send>,
        None,
        span.clone(),
        None,
    );

    let body = PipelineData::ExternalStream {
        stdout: Some(stream),
        stderr: None,
        exit_code: None,
        span: span,
        metadata: None,
        trim_end_newline: false,
    };

    eprintln!("HERE");

    let res = engine
        .eval_closure_with_stream(&closure, vec![Value::record(meta, span)], body, true, false)
        .map_err(|err| LabeledError::new(format!("shell error: {}", err)))
        .unwrap();

    match res {
        PipelineData::Value(value, _) => match value {
            Value::String { val, .. } => {
                tx.blocking_send(Ok(val.into()))
                    .expect("send through channel");
            }
            _ => panic!("Value arm contains an unsupported variant: {:?}", value),
        },

        PipelineData::ListStream(ls, _) => {
            for value in ls.stream {
                let value = match value {
                    Value::String { val, .. } => val,
                    _ => panic!(
                        "ListStream::Value arm contains an unsupported variant: {:?}",
                        value
                    ),
                };
                tx.blocking_send(Ok(value.into()))
                    .expect("send through channel");
            }
        }
        PipelineData::ExternalStream { .. } => panic!("ExternalStream variant"),
        PipelineData::Empty => panic!("Empty variant"),
    }
}

async fn hello(
    engine: &EngineInterface,
    call: &EvaluatedCall,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let span = call.head;
    let mut headers = Record::new();
    for (key, value) in req.headers() {
        headers.insert(
            key.to_string(),
            Value::string(value.to_str().unwrap().to_string(), span),
        );
        eprintln!("key: {:?} {:?}", &key, &value);
    }

    let mut meta = Record::new();
    meta.insert("headers", Value::record(headers, span));
    meta.insert("method", Value::string(req.method().to_string(), span));

    let (tx, mut closure_rx) = mpsc::channel(32);
    let (mut closure_tx, rx) = mpsc::channel(32);

    let mut body = req.into_body();

    tokio::task::spawn(async move {
        while let Some(frame) = body.frame().await {
            eprintln!("FRAME: {:?}", &frame);
            match frame {
                Ok(data) => {
                    // Send the frame data through the channel
                    if let Err(err) = tx.send(Ok(data.into_data().unwrap().to_vec())).await {
                        eprintln!("Error sending frame: {}", err);
                        break;
                    }
                }
                Err(err) => {
                    // Send the error through the channel and break the loop
                    if let Err(err) = tx.send(Err(err)).await {
                        eprintln!("Error sending error: {}", err);
                    }
                    break;
                }
            }
        }
    });

    let engine = engine.clone();
    let call = call.clone();

    std::thread::spawn(move || {
        run_eval(&engine, &call, meta, closure_rx, closure_tx);
    });

    let stream = ReceiverStream::new(rx);
    let stream = stream.map(|data| {
        data.map(|data| {
            eprintln!("streaming");
            Frame::data(bytes::Bytes::from(data))
        })
    });
    let body = StreamBody::new(stream).boxed();
    Ok(Response::new(body))
}

async fn serve(
    engine: &EngineInterface,
    call: &EvaluatedCall,
    mut rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = Path::new("./").join("sock");
    let listener = tokio::net::UnixListener::bind(socket_path)?;

    use tokio::sync::watch;

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        tokio::task::spawn(serve_connection(
                            engine.clone(),
                            call.clone(),
                            TokioIo::new(stream),
                        ));
                    },
                    Err(err) => {
                        eprintln!("Error accepting connection: {}", err);
                    },
                }
            },
            _ = rx.changed() => {
                // TODO: graceful shutdown of inflight connections
                break;
            },
        }
    }
    Ok(())
}

async fn serve_connection<T: std::marker::Unpin + tokio::io::AsyncWrite + tokio::io::AsyncRead>(
    engine: EngineInterface,
    call: EvaluatedCall,
    io: TokioIo<T>,
) {
    if let Err(err) = http1::Builder::new()
        .serve_connection(io, service_fn(|req| hello(&engine, &call, req)))
        .await
    {
        // Match against the error kind to selectively ignore `NotConnected` errors
        if let Some(std::io::ErrorKind::NotConnected) = err.source().and_then(|source| {
            source
                .downcast_ref::<std::io::Error>()
                .map(|io_err| io_err.kind())
        }) {
            // Silently ignore the NotConnected error
        } else {
            // Handle or log other errors
            eprintln!("Error serving connection: {:?}", err);
        }
    }
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}
