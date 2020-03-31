#[macro_use]
extern crate lazy_static;

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{pin_mut, StreamExt};
use std::thread;
use tokio::net::{TcpListener, TcpStream};
use tungstenite::protocol::Message;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use std::convert::Infallible;

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

static INDEX_HTML: &'static [u8] = br#"
<!DOCTYPE html>
<html>
	<head>
		<meta charset="utf-8">
	</head>
	<body>
      <pre id="messages"></pre>
      <script>
        var socket = new WebSocket("ws://" + location.hostname + ":8001" + "/ws");
        socket.onmessage = function (event) {
          var messages = document.getElementById("messages");
          messages.append(event.data + "\n");
        };
		</script>
	</body>
</html>
    "#;

lazy_static! {
    static ref PEERS: PeerMap = PeerMap::new(Mutex::new(HashMap::new()));
}

async fn handle_connection(raw_stream: TcpStream, addr: SocketAddr) {
    println!("Incoming TCP connection from: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");

    println!("WebSocket connection established: {}", addr);

    let (tx, rx) = unbounded();
    PEERS.lock().unwrap().insert(addr, tx);

    let (outgoing, _) = ws_stream.split();
    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(receive_from_others);
    receive_from_others.await.unwrap();

    println!("{} disconnected", &addr);

    PEERS.lock().unwrap().remove(&addr);
}

#[tokio::main]
async fn main() {
    tokio::spawn(start_http_server());

    let mut reader = my_reader::BufReader::open("/home/evaldas/userid.txt").unwrap();
    let mut buffer = String::new();
    let addr = "127.0.0.1:8001";
    let try_socket = TcpListener::bind(&addr).await;
    let mut listener = try_socket.expect("Failed to bind");

    println!("Listening on: {}", addr);

    thread::spawn(move || loop {
        while let Some(line) = reader.read_line(&mut buffer) {
            let message = line.unwrap();
            let peers = PEERS.lock().unwrap();

            let broadcast_recipients = peers.iter().map(|(_, ws_sink)| ws_sink);

            for recp in broadcast_recipients {
                recp.unbounded_send(Message::Text(message.clone())).unwrap();
            }
        }
    });

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(stream, addr));
    }
}

async fn start_http_server() {
    println!("Starting http server");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(front_page)) });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) = server.await {
        println!("server error: {}", e);
    }
}

async fn front_page(_req: Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::new(INDEX_HTML.into()))
}

mod my_reader {
    use std::{
        fs::File,
        io::{self, prelude::*},
    };

    pub struct BufReader {
        reader: io::BufReader<File>,
    }

    impl BufReader {
        pub fn open(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
            let file = File::open(path)?;
            let reader = io::BufReader::new(file);

            Ok(Self { reader })
        }

        pub fn read_line<'buf>(
            &mut self,
            buffer: &'buf mut String,
        ) -> Option<io::Result<&'buf mut String>> {
            buffer.clear();

            self.reader
                .read_line(buffer)
                .map(|u| if u == 0 { None } else { Some(buffer) })
                .transpose()
        }
    }
}
