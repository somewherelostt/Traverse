use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
struct Room {
    #[allow(dead_code)]
    code: String,
    files: Vec<FileEntry>,
    sender_ip: Option<String>,
    #[allow(dead_code)]
    pending_downloads: Vec<String>,
    uploaded_files: HashMap<String, Vec<u8>>, // Store uploaded file data
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
            } else if path.starts_with("/download/") {
                handle_download(stream, path, rooms)
            } else {
                send_404(stream)
            }
        }
        "POST" => {
            if path.starts_with("/upload/") {
                handle_upload(stream, path, &request, rooms)
            } else {
                send_404(stream)
            }
        }
        _ => send_404(stream),
    }
}

fn handle_register(stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    if let Some(query) = path.split('?').nth(1) {
        let mut room_code = None;
        let mut portal_id = None;
        let mut file_name = None;
        let mut file_size = None;
        let sender_ip = stream.peer_addr().ok().map(|addr| addr.ip().to_string());
        
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
                sender_ip: sender_ip.clone(),
                pending_downloads: Vec::new(),
                uploaded_files: HashMap::new(),
            });
            
            // Update sender IP if not set
            if room_entry.sender_ip.is_none() {
                room_entry.sender_ip = sender_ip;
            }
            
            // Add file if not already present
            if !room_entry.files.iter().any(|f| f.name == file) {
                room_entry.files.push(FileEntry {
                    name: file.to_string(),
                    size: size.to_string(),
                    hash: portal.to_string(),
                });
            }
            
            println!("Portal {} registered file {} in room {} from IP {:?}", 
                portal, file, room, room_entry.sender_ip);
            send_ok_response(stream, "Registered successfully")
        } else {
            send_error_response(stream, "Missing parameters")
        }
    } else {
        send_error_response(stream, "No parameters provided")
    }
}

fn handle_join(stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
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

fn handle_upload(mut stream: TcpStream, path: &str, request: &str, rooms: Rooms) -> std::io::Result<()> {
    // Extract room code and file hash from path like /upload/ROOM/HASH
    let path_parts: Vec<&str> = path.split('/').collect();
    if path_parts.len() >= 4 {
        let room_code = path_parts[2];
        let file_hash = path_parts[3];
        
        // Extract content length from headers
        let mut content_length = 0;
        for line in request.lines() {
            if line.to_lowercase().starts_with("content-length:") {
                if let Some(len_str) = line.split(':').nth(1) {
                    content_length = len_str.trim().parse().unwrap_or(0);
                    break;
                }
            }
        }
        
        if content_length > 0 {
            // Find end of headers
            if let Some(body_start) = request.find("\r\n\r\n") {
                let _headers_len = body_start + 4;
                let mut buffer = vec![0u8; content_length];
                let mut bytes_read = 0;
                
                // Read file data
                while bytes_read < content_length {
                    match stream.read(&mut buffer[bytes_read..]) {
                        Ok(0) => break,
                        Ok(n) => bytes_read += n,
                        Err(_) => break,
                    }
                }
                
                // Store file in room
                let mut rooms = rooms.lock().unwrap();
                if let Some(room) = rooms.get_mut(room_code) {
                    room.uploaded_files.insert(file_hash.to_string(), buffer);
                    println!("File {} uploaded to room {}", file_hash, room_code);
                    return send_ok_response(stream, "File uploaded successfully");
                }
            }
        }
    }
    
    send_error_response(stream, "Upload failed")
}

fn handle_download(mut stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    // Extract room code and file hash from path like /download/ROOM/HASH
    let path_parts: Vec<&str> = path.split('/').collect();
    if path_parts.len() >= 4 {
        let room_code = path_parts[2];
        let file_hash = path_parts[3];
        
        let rooms = rooms.lock().unwrap();
        if let Some(room) = rooms.get(room_code) {
            if let Some(file) = room.files.iter().find(|f| f.hash.starts_with(file_hash)) {
                // Check if file is already uploaded
                if let Some(file_data) = room.uploaded_files.get(&file.hash) {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\n\
                        Content-Type: application/octet-stream\r\n\
                        Content-Disposition: attachment; filename=\"{}\"\r\n\
                        Content-Length: {}\r\n\
                        Access-Control-Allow-Origin: *\r\n\r\n",
                        file.name, file_data.len()
                    );
                    stream.write_all(header.as_bytes())?;
                    return stream.write_all(file_data);
                } else {
                    // File not uploaded yet, show waiting page
                    let html = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                        <html><body style='font-family:Arial;background:linear-gradient(135deg,#667eea,#764ba2);color:white;padding:20px'>\
                        <h1>🚀 Preparing Download...</h1>\
                        <p>The file <strong>{}</strong> is being prepared for download.</p>\
                        <p>Please wait while the sender uploads the file to our servers.</p>\
                        <div style='background:rgba(255,255,255,0.1);padding:15px;margin:20px 0;border-radius:10px'>\
                        <h3>Alternative: Use Traverse App</h3>\
                        <p>For faster transfers, use: <code>./target/release/traverse.exe join {}</code></p>\
                        </div>\
                        <script>setTimeout(() => window.location.reload(), 3000);</script>\
                        </body></html>",
                        file.name, room_code
                    );
                    return stream.write_all(html.as_bytes());
                }
            }
        }
    }
    
    // Send 404 if file not found
    let response = "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h1>File Not Found</h1><p>The requested file was not found.</p></body></html>";
    stream.write_all(response.as_bytes())
}

fn handle_room_web(mut stream: TcpStream, path: &str, rooms: Rooms) -> std::io::Result<()> {
    if let Some(room_code) = path.strip_prefix("/room/") {
        let rooms = rooms.lock().unwrap();
        
        let html = if let Some(room) = rooms.get(room_code) {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                <html><body style='font-family:Arial;background:linear-gradient(135deg,#667eea,#764ba2);color:white;padding:20px'>\
                <h1>🌐 Traverse Room {}</h1>\
                <h2>📁 Available Files:</h2>\
                <div>{}</div>\
                <div style='margin-top:30px;padding:15px;background:rgba(255,255,255,0.1);border-radius:10px'>\
                <h3>📱 How to Download:</h3>\
                <p>• <strong>Local Network:</strong> Use Traverse app with room code: <code style='background:rgba(0,0,0,0.3);padding:2px 6px;border-radius:3px'>{}</code></p>\
                <p>• <strong>Command Line:</strong> <code style='background:rgba(0,0,0,0.3);padding:2px 6px;border-radius:3px'>traverse join {}</code></p>\
                </div>\
                </body></html>",
                room_code,
                room.files.iter().map(|f| format!(
                    "<div style='background:rgba(255,255,255,0.1);padding:15px;margin:10px;border-radius:10px'>\
                    <h3>📄 {}</h3>\
                    <p>📊 Size: {} bytes</p>\
                    <p>🔐 Hash: {}</p>\
                    <div style='margin-top:15px'>\
                    <a href='/download/{}/{}' style='background:#28a745;color:white;text-decoration:none;padding:10px 20px;border-radius:5px;display:inline-block;margin-right:10px'>� Download</a>\
                    <p style='color:#ffd700;margin:10px 0 0 0'>💡 Or use CLI: <code style='background:rgba(0,0,0,0.3);padding:4px 8px;border-radius:3px'>traverse join {}</code></p>\
                    </div>\
                    </div>", 
                    f.name, f.size, f.hash, room_code, f.hash, room_code
                )).collect::<Vec<_>>().join(""),
                room_code,
                room_code
            )
        } else {
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n\
            <html><body style='font-family:Arial;background:linear-gradient(135deg,#667eea,#764ba2);color:white;padding:20px'>\
            <h1>❌ Room Not Found</h1>\
            <p>This room doesn't exist or has expired.</p>\
            </body></html>".to_string()
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