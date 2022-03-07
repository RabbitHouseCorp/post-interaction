extern crate dotenv;

use std::collections::HashMap;
use std::convert::Infallible;
use dotenv::dotenv;
use std::{env, result};
use std::borrow::Borrow;
use std::error::Error;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use serde_json::{Value, json};
use tracing::{span, Level};
use warp::{Filter, Rejection, Reply, hyper::StatusCode};
use crossbeam::sync::WaitGroup;
use futures::task::Spawn;
use crate::routes::interaction::interaction_create::interaction_create;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, RwLock};
use tracing_subscriber::fmt::format::FmtSpan;
use warp::ws::{Message, WebSocket, Ws};
use crate::routes::websocket::websocket_server::{websocket_message};
use futures::{SinkExt, StreamExt, TryFutureExt, TryStreamExt};
use futures::FutureExt;
use rustc_serialize::json::ToJson;
use tracing_subscriber::filter::FilterExt;
use crate::routes::websocket::structures::client::{ClientBot, Interaction};


mod sign_mod;
mod cryptography;
mod routes;

type Clients = Arc<RwLock<HashMap<String, ClientBot>>>;
type Interactions = Arc<RwLock<HashMap<String, Interaction>>>;

#[derive(Serialize)]
struct ErrorMessage {
    code: u16,
    code_error: String,
    message: String,
    error: bool
}
struct ResponseData {
    status_code: u64,
}

// HTTP
const interaction_ping: u64 = 1;

// Interaction UI
const interaction_command: u64 = 2;
const interaction_button: u64 = 3;
const interaction_autocomplete : u64 = 4;
const interaction_modal_submit: u64 = 5;



#[tokio::main]
async fn main() {
    dotenv().ok(); // Load env
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "tracing=info,warp=debug".to_owned());
    let mut clients = Clients::default();
    let mut interactions = Interactions::default();
    tracing_subscriber::fmt()
        .with_max_level(Level::WARN)
        .init();

    let clients = warp::any().map(move || clients.clone());
    let interactions = warp::any().map(move || interactions.clone());
    let pub_key = warp::any().map(move || env::var("PUBLIC_KEY").unwrap().clone());

    let extern_api = warp::path::end().map(|| {
        warp::reply::json(
            &json!({ "status_code": 404, "message": "Not found!", "error": false }).as_object_mut()
        )
    });

    let create_interaction = warp::path("interaction")
        .and(pub_key)
        .and(warp::header::header("X-Signature-Ed25519"))
        .and(warp::header::header("X-Signature-Timestamp"))
        .and(warp::body::content_length_limit(1024 * 900))
        .and(warp::body::json())
        .and(clients.clone())
        .and(interactions.clone())
        .and_then(interaction_create);
    let websocket_support = warp::path("ws_interaction")
        .and(warp::ws())
        .and(warp::header::header("Identification-Id"))
        .and(warp::header::header("Secret"))
        .and(warp::header::header("Public-Key"))
        .and(warp::header::header("Shard-In"))
        .and(warp::header::header("Shard-Total"))
        .and(clients.clone())
        .and(interactions.clone())
        .map(|ws: Ws, id: String, secret: String, pub_key_a: String, shard_in: String, shard_total: String, clients, interactions | {
            ws.on_upgrade(move |socket| websocket_message(socket, clients, id, secret, shard_in.parse().unwrap(), shard_total.parse().unwrap(),
                                                          interactions, (pub_key_a, env::var("KEY_SECRET").unwrap(), env::var("PUBLIC_KEY").unwrap(), env::var("BOTS_DISCORD").unwrap())))

        });
    let routes = warp::any()
        .and(
            extern_api
                .or(websocket_support)
                .or(create_interaction)
        )
        .recover(error_api)
        .with(warp::trace::request())
        .with(warp::cors());

    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
}

#[derive(Debug)]
struct Nope;


async fn error_api(err: Rejection) -> Result<impl Reply, Infallible> {
    let code;
    let message;
    let code_msg;
    
    if err.is_not_found() {
        message = "Could not find route";
        code = StatusCode::NOT_FOUND;
        code_msg = "NOT_FOUND";
    } else if let Some(_DivideByZero) = err.find::<Nope>() {
        code = StatusCode::BAD_REQUEST;
        message = "It was not possible to make your request in the API.";
        code_msg = "BAD_REQUEST_API";
    } else if let Some(e) = err.find::<warp::filters::body::BodyDeserializeError>() {
        message = match e.source() {
            Some(cause) => {
                if cause.to_string().contains("denom") {
                    "There are errors in the metadata, please check it."
                } else {
                    "Invalid metadata."
                }
            }
            None => "Unknown error.",
        };

        code_msg = match e.source() {
            Some(cause) => {
                if cause.to_string().contains("denom") {
                    "FIELD_WRONG"
                } else {
                    "BAD_REQUEST"
                }
            }
            None => "BAD_REQUEST",
        };
        code = StatusCode::BAD_REQUEST;
    } else if let Some(_) = err.find::<warp::reject::MethodNotAllowed>() {

        code = StatusCode::METHOD_NOT_ALLOWED;
        code_msg = "METHOD_NOT_ALLOWED";
        message = "Method for this endpoint is invalid for this action.";
    } else {

        eprintln!("unhandled rejection: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Your request was rejected and therefore the API was unable to process your request.";
        code_msg = "UNHANDLED_REJECTION";
    }

    Ok(warp::reply::with_status(
        warp::reply::json(&ErrorMessage {
        code: code.as_u16(),
        code_error: code_msg.to_string(),
        message: message.into(),
        error: true,
    }), code))
}






