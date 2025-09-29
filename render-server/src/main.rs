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
                    if let Err(e) = handle_client(stream, rooms_clone) {
                        eprintln!("Client error: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("Connection error: {}", e),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, rooms: Rooms) -> std::io::Result<()> {
    let mut buffer = [0; 1024];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() { return Ok(()); }
    
    let parts: Vec<&str> = lines[0].split_whitespace().collect();
    if parts.len() < 2 { return Ok(()); }
    
    match parts[0] {
        "REGISTER" => {
            let room_code = parts[1].to_string();
            let portal_id = parts.get(2).unwrap_or(&"").to_string();
            
            let mut rooms = rooms.lock().unwrap();
            rooms.entry(room_code.clone()).or_insert_with(|| Room {
                code: room_code.clone(),
                files: Vec::new(),
            });
            
            println!("Portal {} registered in room {}", portal_id, room_code);
            stream.write_all(b"OK\n")?;
        }
        "JOIN" => {
            let room_code = parts[1];
            let rooms = rooms.lock().unwrap();
            
            if let Some(room) = rooms.get(room_code) {
                stream.write_all(format!("Joined room: {}\n", room_code).as_bytes())?;
                for file in &room.files {
                    stream.write_all(format!("FILE:{}:{}:{}\n", 
                        file.name, file.size, file.hash).as_bytes())?;
                }
            } else {
                stream.write_all(b"Room not found\n")?;
            }
        }
        _ => {
            stream.write_all(b"Unknown command\n")?;
        }
    }
    Ok(())
}