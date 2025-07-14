use axum::{
    Router,
    routing::get,
    response::{Response, IntoResponse},
    body::Bytes,
    extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade}
};
use axum_extra::TypedHeader;
use tower_http::{services::ServeDir, trace::{DefaultMakeSpan, TraceLayer}};
use std::ops::ControlFlow;
use std::{net::SocketAddr, path::PathBuf};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
use futures::{sink::SinkExt, stream::StreamExt};

#[tokio::main]
async fn main() {
    let fallback_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fallback");
    let app = Router::new()
            .fallback_service(ServeDir::new(fallback_dir).append_index_html_on_directories(true))
            .route("/ws", get(ws_handler))
            .layer(TraceLayer::new_for_http().make_span_with(DefaultMakeSpan::default().include_headers(true)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}

async fn ws_handler(
    ws : WebSocketUpgrade,
    user_agent : Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr) : ConnectInfo<SocketAddr>
) -> impl IntoResponse {
    let agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    println!("{agent} has connected to server with address {addr}");
    ws.on_upgrade(move |socket| {
        handle_socket(socket, addr)
    })
}

async fn handle_socket(mut socket : WebSocket, who : SocketAddr) {
    // sending a ping message
    if socket.send(Message::Ping(Bytes::from_static(&[1,2,3]))).await.is_ok() {
        println!("Pinged {who}");
    } else {
        println!("Could not send ping to {who}");
    }

    //receiving a single message

    if let Some(msg) = socket.recv().await {
        if let Ok(message) = msg {
            if process_message(message, who).is_break() {
                println!("Connection aborted because the client wanted to");
                return;
            }
        } else {
            println!("Connection aborted abruptly");
            return;
        }
    }
    
    // splitting the socket because socket implements both sink and stream.. therefore to do both 
    //simultaneously we will have to split it to get two halves one implementing sink and the other stream

    let (mut sender, mut receiver) = socket.split();
    // we now spawn tasks for sending and receiving
    let mut send_task = tokio::spawn(async move {
        let count = 20;
        for i in 0..20 {
            if sender.send(Message::Text(format!("Hello from Abhinav").into())).await.is_err() {
                return i;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        println!("Sending close message to the client");
        if let Err(e) = sender.send(Message::Close(Some(CloseFrame{
            code: axum::extract::ws::close_code::NORMAL,
            reason: Utf8Bytes::from_static("See you next time"),
        }))).await {
            println!("Failed to send close frame probably because the connection has already closed");
        }
        return count;
        // the above code just sends the closeframe but does not wait for the client to send it back
        // so this is not a graceful shutdown but a unilateral shutdown
        // this will only terminate the future not the entire program
    });

    // spawning a task to receive messages 
    let mut receive_task = tokio::spawn(async move {
        let mut number = 0;
        while let Some(Ok(msg)) = receiver.next().await {
            number = number + 1;
            if process_message(msg, who).is_break() {
                break;
            }
        }
        return number;
    });

    // we also have to make sure that if any of the tasks exits the other exits as well
    
    tokio::select! {
        a = (&mut send_task) => {
            match a {
                Ok(m) => println!("Completed successfully: {m} messages have been sent"),
                Err(n) => println!("Error sending message {n:?}"),
            }
            receive_task.abort();
        },
        b = (&mut receive_task) => {
            match b {
                Ok(o) => println!("Completed successfully: {o} messages received"),
                Err(p) => println!("Error receiving message {p:?}"),
            }
            send_task.abort();
        }
    }
    println!("Websocket connected has been terminated");
}

fn process_message(message : Message , who : SocketAddr) -> ControlFlow<(), ()> {
    match message {
        Message::Text(t) => {
            println!("Received text message {t:?}");
        }
        Message::Binary(b) => {
            println!("Received bytes {b:?}");
        }
        Message::Pong(p) => {
            println!("Received pong message {p:?}");
        }
        Message::Close(c) => {
            if let Some(cf) = c {
                println!("Received closing frame with code {} and reason {}", cf.code, cf.reason);
            } else {
                println!("Client somehow sent close message without the frame");
            }
            return ControlFlow::Break(());
        }
        // axum automatically handles ping message from the client side by responding with pong
        // but we can get the data
        Message::Ping(p) => {
            println!("Received ping message {p:?}");
        }
    }
    return ControlFlow::Continue(());
}