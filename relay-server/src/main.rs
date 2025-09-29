use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

// Simple relay server for Traverse
// Handles room-based file sharing across the internet

struct Room {
    files: Vec<FileEntry>,
    participants: Vec<String>,
}

struct FileEntry {
    filename: String,
    size: u64,
    hash: String,
    #[allow(dead_code)]
    sender_id: String,
}

struct RelayServer {
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    clients: Arc<Mutex<HashMap<String, TcpStream>>>,
}

impl RelayServer {
    fn new() -> Self {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("0.0.0.0:80")?;
        println!("🌍 Traverse Relay Server running on port 80");

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let rooms = Arc::clone(&self.rooms);
                    let clients = Arc::clone(&self.clients);
                    
                    thread::spawn(move || {
                        handle_client(stream, rooms, clients);
                    });
                }
                Err(e) => {
                    eprintln!("Connection failed: {}", e);
                }
            }
        }

        Ok(())
    }
}

fn handle_client(
    mut stream: TcpStream,
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    _clients: Arc<Mutex<HashMap<String, TcpStream>>>,
) {
    let mut buffer = [0; 1024];
    
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break, // Connection closed
            Ok(n) => {
                let message = String::from_utf8_lossy(&buffer[..n]);
                let response = process_message(&message, &rooms);
                
                if let Err(_) = stream.write_all(response.as_bytes()) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn process_message(
    message: &str,
    rooms: &Arc<Mutex<HashMap<String, Room>>>,
) -> String {
    let parts: Vec<&str> = message.trim().split_whitespace().collect();
    
    if parts.is_empty() {
        return "ERROR: Empty message\n".to_string();
    }

    match parts[0] {
        "REGISTER" => {
            if parts.len() < 3 {
                return "ERROR: Invalid REGISTER format\n".to_string();
            }
            
            let room_code = parts[1];
            let portal_id = parts[2];
            
            let mut rooms_lock = rooms.lock().unwrap();
            let room = rooms_lock.entry(room_code.to_string()).or_insert(Room {
                files: Vec::new(),
                participants: Vec::new(),
            });
            
            if !room.participants.contains(&portal_id.to_string()) {
                room.participants.push(portal_id.to_string());
            }
            
            format!("OK: Registered in room {}\n", room_code)
        }
        "JOIN" => {
            if parts.len() < 2 {
                return "ERROR: Invalid JOIN format\n".to_string();
            }
            
            let room_code = parts[1];
            let rooms_lock = rooms.lock().unwrap();
            
            if let Some(room) = rooms_lock.get(room_code) {
                let mut response = format!("ROOM: {} ({} participants)\n", room_code, room.participants.len());
                
                for file in &room.files {
                    response.push_str(&format!(
                        "FILE:{}:{}:{}\n",
                        file.filename, file.size, file.hash
                    ));
                }
                
                if room.files.is_empty() {
                    response.push_str("INFO: No files available in this room\n");
                }
                
                response
            } else {
                "ERROR: Room not found\n".to_string()
            }
        }
        "FILE_LIST" => {
            // Handle file listing from senders
            if parts.len() < 5 {
                return "ERROR: Invalid FILE_LIST format\n".to_string();
            }
            
            let room_code = parts[1];
            let filename = parts[2];
            let size = parts[3].parse::<u64>().unwrap_or(0);
            let hash = parts[4];
            let sender_id = parts.get(5).unwrap_or(&"unknown").to_string();
            
            let mut rooms_lock = rooms.lock().unwrap();
            if let Some(room) = rooms_lock.get_mut(room_code) {
                room.files.push(FileEntry {
                    filename: filename.to_string(),
                    size,
                    hash: hash.to_string(),
                    sender_id,
                });
                
                "OK: File registered\n".to_string()
            } else {
                "ERROR: Room not found\n".to_string()
            }
        }
        _ => "ERROR: Unknown command\n".to_string(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Traverse Relay Server...");
    
    let server = RelayServer::new();
    server.start()?;
    
    Ok(())
}