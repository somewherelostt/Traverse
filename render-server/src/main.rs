use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
struct Room {
    code: String,
    files: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
struct FileEntry {
    name: String,
    size: String,
    hash: String,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

fn main() -> std::io::Result<()> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    
    println!("Traverse Relay Server running on {}", addr);
    let listener = TcpListener::bind(addr)?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let rooms_clone = Arc::clone(&rooms);
                thread::spawn(move || {
                    if let Err(e) = handle_http_client(stream, rooms_clone) {
                        eprintln!("Client error: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("Connection error: {}", e),
        }
    }
    Ok(())
}

fn handle_http_client(mut stream: TcpStream, rooms: Rooms) -> std::io::Result<()> {
    let mut buffer = [0; 1024];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() { return Ok(()); }
    
    let request_line = lines[0];
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 { return Ok(()); }
    
    let method = parts[0];
    let path = parts[1];
    
    match method {
        "GET" => {
            if path.starts_with("/register") {
                handle_register(stream, path, rooms)
            } else if path.starts_with("/join") {
                handle_join(stream, path, rooms)
            } else if path.starts_with("/room/") {
                handle_room_web(stream, path, rooms)
            } else {
                send_404(stream)
            }
        }
        _ => send_404(stream),
    }
}

fn handle_register(mut stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    if let Some(query) = path.split('?').nth(1) {
        let mut room_code = None;
        let mut portal_id = None;
        let mut file_name = None;
        let mut file_size = None;
        
        for param in query.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                match key {
                    "room" => room_code = Some(value),
                    "portal" => portal_id = Some(value),
                    "file" => file_name = Some(value),
                    "size" => file_size = Some(value),
                    _ => {}
                }
            }
        }
        
        if let (Some(room), Some(portal), Some(file), Some(size)) = (room_code, portal_id, file_name, file_size) {
            let mut rooms = rooms.lock().unwrap();
            let room_entry = rooms.entry(room.to_string()).or_insert_with(|| Room {
                code: room.to_string(),
                files: Vec::new(),
            });
            
            // Add file if not already present
            if !room_entry.files.iter().any(|f| f.name == file) {
                room_entry.files.push(FileEntry {
                    name: file.to_string(),
                    size: size.to_string(),
                    hash: portal.to_string(),
                });
            }
            
            println!("Portal {} registered file {} in room {}", portal, file, room);
            send_ok_response(stream, "Registered successfully")
        } else {
            send_error_response(stream, "Missing parameters")
        }
    } else {
        send_error_response(stream, "No parameters provided")
    }
}

fn handle_join(mut stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    if let Some(query) = path.split('?').nth(1) {
        if let Some(room_code) = query.strip_prefix("room=") {
            let rooms = rooms.lock().unwrap();
            
            if let Some(room) = rooms.get(room_code) {
                let mut response = format!("Joined room: {}\n", room_code);
                for file in &room.files {
                    response.push_str(&format!("FILE:{}:{}:{}\n", 
                        file.name, file.size, file.hash));
                }
                send_ok_response(stream, &response)
            } else {
                send_error_response(stream, "Room not found")
            }
        } else {
            send_error_response(stream, "Invalid room parameter")
        }
    } else {
        send_error_response(stream, "No room specified")
    }
}

fn handle_room_web(mut stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    if let Some(room_code) = path.strip_prefix("/room/") {
        let rooms = rooms.lock().unwrap();
        
        let html = if let Some(room) = rooms.get(room_code) {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                <html><body style='font-family:Arial;background:linear-gradient(135deg,#667eea,#764ba2);color:white;padding:20px'>\
                <h1>Traverse Room {}</h1>\
                <h2>Available Files:</h2>\
                <div>{}</div>\
                </body></html>",
                room_code,
                room.files.iter().map(|f| format!(
                    "<div style='background:rgba(255,255,255,0.1);padding:15px;margin:10px;border-radius:10px'>\
                    <h3>{}</h3><p>Size: {} bytes</p>\
                    <p>Hash: {}</p></div>", 
                    f.name, f.size, f.hash
                )).collect::<Vec<_>>().join("")
            )
        } else {
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n\
            <html><body><h1>Room Not Found</h1></body></html>".to_string()
        };
        
        stream.write_all(html.as_bytes())
    } else {
        send_404(stream)
    }
}

fn send_ok_response(mut stream: TcpStream, message: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n{}",
        message
    );
    stream.write_all(response.as_bytes())
}

fn send_error_response(mut stream: TcpStream, message: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nError: {}",
        message
    );
    stream.write_all(response.as_bytes())
}

fn send_404(mut stream: TcpStream) -> std::io::Result<()> {
    let response = "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h1>404 Not Found</h1></body></html>";
    stream.write_all(response.as_bytes())
}