#![allow(warnings)]

use std::path::Path;

use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};

use nu_protocol::engine::Closure;
use nu_protocol::{LabeledError, PipelineData, Signature, Span, Spanned, SyntaxShape, Type, Value};

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
            .input_output_type(Type::Nothing, Type::Any)
    }

    fn run(
        &self,
        plugin: &HTTPPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        plugin.runtime.block_on(async move {
            let _ = serve(engine, call).await;
        });

        let span = call.head;
        let value = Value::string("hello", span);
        let body = PipelineData::Value(value, None);
        return Ok(body);
    }
}

use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

async fn hello(
    _engine: &EngineInterface,
    call: &EvaluatedCall,
    _: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}

async fn serve(
    engine: &EngineInterface,
    call: &EvaluatedCall,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = Path::new("./").join("sock");
    let listener = tokio::net::UnixListener::bind(socket_path).unwrap();

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let engine = engine.clone();
        let call = call.clone();
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(|req| hello(&engine, &call, req)))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}
