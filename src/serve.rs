use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::StreamBody;
use hyper::body::{Bytes, Frame};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use nu_plugin::EngineInterface;
use nu_protocol::engine::Closure;
use nu_protocol::{
    ByteStream, ByteStreamType, HandlerGuard, LabeledError, PipelineData, Record, ShellError, Span,
    Spanned, Value,
};
use std::error::Error;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

fn run_eval(
    engine: &EngineInterface,
    span: Span,
    closure: Spanned<Closure>,
    meta: Record,
    mut rx: mpsc::Receiver<Result<Vec<u8>, hyper::Error>>,
    tx: mpsc::Sender<Result<Vec<u8>, ShellError>>,
) {
    let stream = ByteStream::from_fn(
        span,
        engine.signals().clone(),
        ByteStreamType::Unknown,
        move |buffer: &mut Vec<u8>| match rx.blocking_recv() {
            Some(Ok(bytes)) => {
                buffer.extend_from_slice(&bytes);
                Ok(true)
            }
            Some(Err(err)) => Err(ShellError::LabeledError(Box::new(LabeledError::new(
                format!("Read error: {}", err),
            )))),
            None => Ok(false),
        },
    );

    let res = engine
        .eval_closure_with_stream(
            &closure,
            vec![Value::record(meta, span)],
            stream.into(),
            true,
            false,
        )
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
            for value in ls.into_inner() {
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
        /*
        PipelineData::ExternalStream { stdout, .. } => {
            if let Some(stdout) = stdout {
                for value in stdout.stream {
                    tx.blocking_send(value).expect("send through channel");
                }
            }
        }
        */
        PipelineData::ByteStream(_, _) => {
            panic!()
        }
        PipelineData::Empty => (),
    }
}

async fn process_req(
    engine: &EngineInterface,
    span: Span,
    closure: Spanned<Closure>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, ShellError>>, hyper::Error> {
    let mut headers = Record::new();
    for (key, value) in req.headers() {
        headers.insert(
            key.to_string(),
            Value::string(value.to_str().unwrap().to_string(), span),
        );
    }

    let uri = req.uri();
    let method = req.method().to_string();
    let path = uri.path().to_string();
    let query = uri.query();
    let query_params = if let Some(qs) = query {
        let pairs = qs
            .split("&")
            .map(|kv| {
                Value::list(
                    kv.split("=").map(|s| Value::string(s, span)).collect(),
                    span,
                )
            })
            .collect();
        Value::list(pairs, span)
    } else {
        Value::list(vec![], span)
    };

    println!("Received {} {} ({:?})", method, path, query);

    let mut meta = Record::new();
    meta.insert("headers", Value::record(headers, span));
    meta.insert("method", Value::string(method, span));
    meta.insert("path", Value::string(path, span));
    meta.insert("params", query_params);

    let (tx, closure_rx) = mpsc::channel(32);
    let (closure_tx, rx) = mpsc::channel(32);

    let mut body = req.into_body();

    tokio::task::spawn(async move {
        while let Some(frame) = body.frame().await {
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

    std::thread::spawn(move || {
        run_eval(&engine, span, closure, meta, closure_rx, closure_tx);
    });

    let stream = ReceiverStream::new(rx);
    let stream = stream.map(|data| data.map(|data| Frame::data(bytes::Bytes::from(data))));
    let body = StreamBody::new(stream).boxed();
    Ok(Response::new(body))
}

pub(crate) async fn serve(
    mut ctrlc_rx: tokio::sync::watch::Receiver<bool>,
    _guard: HandlerGuard,
    engine: &EngineInterface,
    span: Span,
    closure: Spanned<Closure>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(addr).await?;

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        tokio::task::spawn(serve_connection(
                            engine.clone(),
                            span.clone(),
                            closure.clone(),
                            TokioIo::new(stream),
                        ));
                    },
                    Err(err) => {
                        eprintln!("Error accepting connection: {}", err);
                    },
                }
            },
            _ = ctrlc_rx.changed() => {
                println!("Received Ctrl+c");
                // TODO: graceful shutdown of inflight connections
                break;
            },
        }
    }
    Ok(())
}

async fn serve_connection<T: std::marker::Unpin + tokio::io::AsyncWrite + tokio::io::AsyncRead>(
    engine: EngineInterface,
    span: Span,
    closure: Spanned<Closure>,
    io: TokioIo<T>,
) {
    if let Err(err) = hyper::server::conn::http1::Builder::new()
        .serve_connection(
            io,
            hyper::service::service_fn(|req| process_req(&engine, span, closure.clone(), req)),
        )
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
