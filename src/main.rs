use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sha2::{Sha256, Digest};
use qr2term::print_qr;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

const CHUNK_SIZE: usize = 64 * 1024;
const DISCOVERY_PORT: u16 = 8765;
const TRANSFER_PORT: u16 = 8766;
const WEB_PORT: u16 = 8767;
const RELAY_SERVER: &str = "traverse-yt17.onrender.com";

#[derive(Clone, Debug)]
struct FileInfo {
    name: String,
    size: u64,
    chunk_count: usize,
    hash: String,
    room_code: String,
}

#[derive(Clone)]
struct Portal {
    file_info: FileInfo,
    file_path: String,
}

struct TraverseNode {
    portals: Arc<Mutex<HashMap<String, Portal>>>,
}

impl TraverseNode {
    fn new() -> Self {
        Self { portals: Arc::new(Mutex::new(HashMap::new())) }
    }

    fn send_file(&self, file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let path = Path::new(file_path);
        if !path.exists() { return Err("File not found".into()); }

        let file_size = path.metadata()?.len();
        let chunk_count = ((file_size as f64) / (CHUNK_SIZE as f64)).ceil() as usize;
        
        let mut file = File::open(file_path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; CHUNK_SIZE];
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 { break; }
            hasher.update(&buffer[..bytes_read]);
        }
        let file_hash = format!("{:x}", hasher.finalize());
        let room_code = format!("{:06}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() % 999999);

        let file_info = FileInfo {
            name: path.file_name().unwrap().to_string_lossy().to_string(),
            size: file_size, chunk_count, hash: file_hash.clone(), room_code: room_code.clone(),
        };

        let portal = Portal { file_info: file_info.clone(), file_path: file_path.to_string() };
        let portal_id = file_hash[..8].to_string();
        self.portals.lock().unwrap().insert(portal_id.clone(), portal.clone());

        println!("TRAVERSE PORTAL ACTIVE");
        println!("Room: {} | File: {}", room_code, file_info.name);
        println!("Size: {} ({} chunks)", format_bytes(file_info.size), file_info.chunk_count);

        self.start_services(portal_id.clone(), room_code.clone())?;

        let local_ip = get_local_ip().unwrap_or("127.0.0.1".to_string());
        println!("Local: http://{}:{}", local_ip, WEB_PORT);
        println!("Internet: https://{}/room/{}", RELAY_SERVER, room_code);
        
        let _ = print_qr(&format!("http://{}:{}", local_ip, WEB_PORT));
        Ok(portal_id)
    }

    fn start_services(&self, portal_id: String, room_code: String) -> Result<(), Box<dyn std::error::Error>> {
        let portals = Arc::clone(&self.portals);
        
        // Registration service
        let (pid, rc, ps) = (portal_id.clone(), room_code.clone(), Arc::clone(&portals));
        thread::spawn(move || {
            if let Ok(portals) = ps.lock() {
                if let Some(portal) = portals.get(&pid) {
                    let url = format!("https://{}/register?room={}&portal={}&file={}&size={}", 
                        RELAY_SERVER, rc, pid, portal.file_info.name, portal.file_info.size);
                    let _ = std::process::Command::new("curl").arg("-s").arg(&url).output();
                }
            }
        });

        // Upload service
        let (pid2, rc2, ps2) = (portal_id.clone(), room_code.clone(), Arc::clone(&portals));
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            if let Ok(portals) = ps2.lock() {
                if let Some(portal) = portals.get(&pid2) {
                    let url = format!("https://{}/upload/{}/{}", RELAY_SERVER, rc2, portal.file_info.hash);
                    let _ = std::process::Command::new("curl").arg("-X").arg("POST")
                        .arg("-H").arg("Content-Type: application/octet-stream")
                        .arg("--data-binary").arg(&format!("@{}", portal.file_path)).arg(&url).output();
                }
            }
        });

        // Discovery service
        let (pid3, ps3) = (portal_id.clone(), Arc::clone(&portals));
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        if let Ok(portals) = ps3.lock() {
                            if let Some(portal) = portals.get(&pid3) {
                                let response = format!("PORTAL:{}:{}:{}:{}:{}\n", 
                                    pid3, portal.file_info.name, portal.file_info.size, 
                                    portal.file_info.chunk_count, portal.file_info.hash);
                                let _ = stream.write_all(response.as_bytes());
                            }
                        }
                    }
                }
            }
        });

        // Transfer service
        let ps4 = Arc::clone(&portals);
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", TRANSFER_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let mut buffer = [0; 256];
                        if let Ok(n) = stream.read(&mut buffer) {
                            let request = String::from_utf8_lossy(&buffer[..n]);
                            if let Some(chunk_idx) = request.strip_prefix("CHUNK:") {
                                if let Ok(idx) = chunk_idx.trim().parse::<usize>() {
                                    if let Ok(portals) = ps4.lock() {
                                        for portal in portals.values() {
                                            if let Ok(chunk_data) = read_chunk(&portal.file_path, idx) {
                                                let _ = stream.write_all(&chunk_data);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Web service
        let ps5 = Arc::clone(&portals);
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", WEB_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let mut buffer = [0; 1024];
                        if let Ok(n) = stream.read(&mut buffer) {
                            let request = String::from_utf8_lossy(&buffer[..n]);
                            let first_line = request.lines().next().unwrap_or("");
                            
                            if first_line.starts_with("GET /download/") {
                                if let Some(hash_part) = first_line.split("/download/").nth(1) {
                                    let hash = hash_part.split_whitespace().next().unwrap_or("");
                                    if let Ok(portals) = ps5.lock() {
                                        if let Some(portal) = portals.values().find(|p| p.file_info.hash.starts_with(hash)) {
                                            if let Ok(mut file) = File::open(&portal.file_path) {
                                                let mut file_data = Vec::new();
                                                if file.read_to_end(&mut file_data).is_ok() {
                                                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\n\r\n", portal.file_info.name, file_data.len());
                                                    let _ = stream.write_all(header.as_bytes());
                                                    let _ = stream.write_all(&file_data);
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\nFile not found");
                                }
                            } else {
                                let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Traverse Portal</h1>";
                                let _ = stream.write_all(response.as_bytes());
                                if let Ok(portals) = ps5.lock() {
                                    for portal in portals.values() {
                                        let html = format!("<div><h3>{}</h3><p>Size: {}</p><a href='/download/{}'>Download</a></div>", portal.file_info.name, format_bytes(portal.file_info.size), portal.file_info.hash);
                                        let _ = stream.write_all(html.as_bytes());
                                    }
                                }
                                let _ = stream.write_all(b"</body></html>");
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn discover_portals(&self) -> Result<HashMap<String, (SocketAddr, FileInfo)>, Box<dyn std::error::Error>> {
        let mut portals = HashMap::new();
        for i in 100..200 {
            if let Ok(addr) = format!("192.168.1.{}:{}", i, DISCOVERY_PORT).parse::<SocketAddr>() {
                if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(100)) {
                    let mut buffer = [0; 512];
                    if let Ok(n) = stream.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        if let Some(portal_data) = response.strip_prefix("PORTAL:") {
                            let parts: Vec<&str> = portal_data.trim().split(':').collect();
                            if parts.len() >= 5 {
                                let file_info = FileInfo {
                                    name: parts[1].to_string(), size: parts[2].parse().unwrap_or(0),
                                    chunk_count: parts[3].parse().unwrap_or(0), hash: parts[4].to_string(), room_code: String::new(),
                                };
                                portals.insert(parts[0].to_string(), (addr, file_info));
                            }
                        }
                    }
                }
            }
        }
        Ok(portals)
    }

    fn receive_file(&self, _portal_id: &str, transfer_addr: SocketAddr, file_info: &FileInfo) -> Result<(), Box<dyn std::error::Error>> {
        println!("Downloading: {} ({} chunks)", file_info.name, file_info.chunk_count);
        let pb = ProgressBar::new(file_info.chunk_count as u64);
        pb.set_style(ProgressStyle::default_bar().template("{bar:40} {pos}/{len}")?);
        let output_path = format!("downloaded_{}", file_info.name);
        let mut output_file = OpenOptions::new().create(true).write(true).truncate(true).open(&output_path)?;
        
        for chunk_idx in 0..file_info.chunk_count {
            if let Ok(mut stream) = TcpStream::connect_timeout(&transfer_addr, Duration::from_secs(5)) {
                let request = format!("CHUNK:{}", chunk_idx);
                stream.write_all(request.as_bytes())?;
                let mut chunk_data = Vec::new();
                stream.read_to_end(&mut chunk_data)?;
                if !chunk_data.is_empty() {
                    output_file.write_all(&chunk_data)?;
                    pb.inc(1);
                }
            }
        }
        pb.finish_with_message("Download complete");
        println!("Saved to: {}", output_path);
        Ok(())
    }

    fn join_room(&self, room_code: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Joining room: {}", room_code);
        let url = format!("https://{}/join?room={}", RELAY_SERVER, room_code);
        let output = std::process::Command::new("curl").arg("-s").arg(&url).output()
            .unwrap_or_else(|_| std::process::Output { status: std::process::ExitStatus::default(), stdout: Vec::new(), stderr: Vec::new() });
        
        let response = String::from_utf8_lossy(&output.stdout);
        if response.contains("Joined room") || response.contains("FILE:") {
            println!("Connected to relay server");
            let mut available_files = Vec::new();
            for line in response.lines() {
                if line.starts_with("FILE:") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 4 {
                        let file_info = FileInfo {
                            name: parts[1].to_string(), size: parts[2].parse().unwrap_or(0),
                            chunk_count: ((parts[2].parse::<u64>().unwrap_or(0) as f64) / (CHUNK_SIZE as f64)).ceil() as usize,
                            hash: parts[3].to_string(), room_code: room_code.to_string(),
                        };
                        available_files.push(file_info);
                        println!("{}. {}: {} - Available", available_files.len(), parts[1], format_bytes(parts[2].parse().unwrap_or(0)));
                    }
                }
            }
            
            if !available_files.is_empty() {
                print!("Select file (1-{}): ", available_files.len());
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                
                if let Ok(choice) = input.trim().parse::<usize>() {
                    if choice > 0 && choice <= available_files.len() {
                        let selected_file = &available_files[choice - 1];
                        let download_url = format!("https://{}/download/{}/{}", RELAY_SERVER, room_code, selected_file.hash);
                        let output_path = format!("downloaded_{}", selected_file.name);
                        
                        if std::process::Command::new("curl").arg("-L").arg("-o").arg(&output_path).arg(&download_url).status()?.success() {
                            println!("Downloaded to: {}", output_path);
                        } else {
                            println!("Download failed");
                        }
                    }
                }
            }
        } else {
            println!("Room not found or connection failed");
        }
        Ok(())
    }
}

fn read_chunk(file_path: &str, chunk_index: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut file = File::open(file_path)?;
    file.seek(SeekFrom::Start((chunk_index * CHUNK_SIZE) as u64))?;
    
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let bytes_read = file.read(&mut buffer)?;
    buffer.truncate(bytes_read);
    Ok(buffer)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < 3 { size /= 1024.0; unit += 1; }
    if unit == 0 { format!("{} {}", bytes, UNITS[unit]) } 
    else { format!("{:.1} {}", size, UNITS[unit]) }
}

fn get_local_ip() -> Option<String> {
    if let Ok(output) = std::process::Command::new("ipconfig").output() {
        let output_str = String::from_utf8_lossy(&output.stdout);
        for line in output_str.lines() {
            if line.contains("IPv4 Address") && line.contains("192.168") {
                if let Some(ip) = line.split(':').nth(1) {
                    return Some(ip.trim().to_string());
                }
            }
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        println!("TRAVERSE v2.0\nInternet P2P File Sharing\n");
        println!("Usage:");
        println!("  {} send <file>     - Share globally", args[0]);
        println!("  {} recv            - Local discovery", args[0]);
        println!("  {} join <code>     - Join internet room", args[0]);
        return Ok(());
    }

    let node = TraverseNode::new();

    match args[1].as_str() {
        "send" => {
            if args.len() < 3 { 
                println!("Error: Specify file to send"); 
                return Ok(()); 
            }
            node.send_file(&args[2])?;
            loop { thread::sleep(Duration::from_secs(1)); }
        }
        "join" => {
            if args.len() < 3 { 
                println!("Error: Specify room code"); 
                return Ok(()); 
            }
            node.join_room(&args[2])?;
        }
        "recv" => {
            println!("Discovering local portals...");
            let portals = node.discover_portals()?;
            
            if portals.is_empty() {
                println!("No portals found on local network");
                return Ok(());
            }

            println!("Available files:");
            for (i, (portal_id, (_addr, file_info))) in portals.iter().enumerate() {
                println!("{}. {} - {} ({})", i + 1, file_info.name, format_bytes(file_info.size), portal_id);
            }

            print!("Select file (1-{}): ", portals.len());
            std::io::stdout().flush().unwrap();
            
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            
            if let Ok(choice) = input.trim().parse::<usize>() {
                if choice > 0 && choice <= portals.len() {
                    let (portal_id, (addr, file_info)) = portals.iter().nth(choice - 1).unwrap();
                    let transfer_addr = SocketAddr::new(addr.ip(), TRANSFER_PORT);
                    node.receive_file(portal_id, transfer_addr, file_info)?;
                }
            }
        }
        _ => println!("Unknown command"),
    }
    
    Ok(())
}