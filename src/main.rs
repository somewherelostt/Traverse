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
    #[allow(dead_code)]
    room_code: String,
}

#[derive(Clone)]
struct Portal {
    file_info: FileInfo,
    file_path: String,
    #[allow(dead_code)]
    peers: Arc<Mutex<Vec<SocketAddr>>>,
}

struct TraverseNode {
    portals: Arc<Mutex<HashMap<String, Portal>>>,
    chunk_cache: Arc<Mutex<HashMap<String, HashMap<usize, Vec<u8>>>>>,
}

impl TraverseNode {
    fn new() -> Self {
        Self {
            portals: Arc::new(Mutex::new(HashMap::new())),
            chunk_cache: Arc::new(Mutex::new(HashMap::new())),
        }
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

        let portal = Portal {
            file_info: file_info.clone(),
            file_path: file_path.to_string(),
            peers: Arc::new(Mutex::new(Vec::new())),
        };

        let portal_id = file_hash[..8].to_string();
        self.portals.lock().unwrap().insert(portal_id.clone(), portal.clone());

        println!("{}", "TRAVERSE PORTAL ACTIVE".bright_cyan().bold());
        println!("Room: {} | File: {}", room_code.bright_yellow(), file_info.name.bright_green());
        println!("Size: {} ({} chunks)", format_bytes(file_info.size).bright_blue(), file_info.chunk_count);

        self.start_services(portal_id.clone(), room_code.clone())?;

        let local_ip = get_local_ip().unwrap_or("127.0.0.1".to_string());
        println!("Local: {}", format!("http://{}:{}", local_ip, WEB_PORT).bright_cyan());
        println!("Internet: {}", format!("https://{}/room/{}", RELAY_SERVER, room_code).bright_magenta());
        
        if let Err(_) = print_qr(&format!("http://{}:{}", local_ip, WEB_PORT)) {
            println!("QR: {}", format!("http://{}:{}", local_ip, WEB_PORT).dimmed());
        }

        Ok(portal_id)
    }

    fn start_services(&self, portal_id: String, room_code: String) -> Result<(), Box<dyn std::error::Error>> {
        let portal_id_relay = portal_id.clone();
        let room_code_relay = room_code.clone();
        let portals_reg = Arc::clone(&self.portals);
        thread::spawn(move || {
            // Get file info from the portal
            let (file_name, file_size) = if let Ok(portals) = portals_reg.lock() {
                if let Some(portal) = portals.get(&portal_id_relay) {
                    (portal.file_info.name.clone(), portal.file_info.size)
                } else {
                    return;
                }
            } else {
                return;
            };
            
            let url = format!("https://{}/register?room={}&portal={}&file={}&size={}", 
                RELAY_SERVER, room_code_relay, portal_id_relay, file_name, file_size);
            
            // Try curl first, then PowerShell as fallback
            if std::process::Command::new("curl").arg("-s").arg(&url).output().is_err() {
                let ps_command = format!("Invoke-WebRequest -Uri '{}' -UseBasicParsing", url);
                let _ = std::process::Command::new("powershell").arg("-Command").arg(ps_command).output();
            }
        });

        let portals_disc = Arc::clone(&self.portals);
        let portal_id_disc = portal_id.clone();
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        if let Ok(portals) = portals_disc.lock() {
                            if let Some(portal) = portals.get(&portal_id_disc) {
                                let response = format!("PORTAL:{}:{}:{}:{}:{}\n", 
                                    portal_id_disc, portal.file_info.name, portal.file_info.size, 
                                    portal.file_info.chunk_count, portal.file_info.hash);
                                let _ = stream.write_all(response.as_bytes());
                            }
                        }
                    }
                }
            }
        });

        let portals_transfer = Arc::clone(&self.portals);
        let cache_transfer = Arc::clone(&self.chunk_cache);
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", TRANSFER_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let mut buffer = [0; 256];
                        if let Ok(n) = stream.read(&mut buffer) {
                            let request = String::from_utf8_lossy(&buffer[..n]);
                            if let Some(chunk_idx) = request.strip_prefix("CHUNK:") {
                                if let Ok(idx) = chunk_idx.trim().parse::<usize>() {
                                    if let Ok(portals) = portals_transfer.lock() {
                                        for portal in portals.values() {
                                            if let Ok(chunk_data) = read_chunk(&portal.file_path, idx) {
                                                let _ = stream.write_all(&chunk_data);
                                                if let Ok(mut cache) = cache_transfer.lock() {
                                                    cache.entry(portal.file_info.hash.clone())
                                                         .or_insert_with(HashMap::new)
                                                         .insert(idx, chunk_data);
                                                }
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

        let portals_web = Arc::clone(&self.portals);
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", WEB_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let mut buffer = [0; 1024];
                        if let Ok(n) = stream.read(&mut buffer) {
                            let request = String::from_utf8_lossy(&buffer[..n]);
                            let first_line = request.lines().next().unwrap_or("");
                            
                            if first_line.starts_with("GET /download/") {
                                // Handle download request
                                if let Some(hash_part) = first_line.split("/download/").nth(1) {
                                    let hash = hash_part.split_whitespace().next().unwrap_or("");
                                    if let Ok(portals) = portals_web.lock() {
                                        if let Some(portal) = portals.values().find(|p| p.file_info.hash.starts_with(hash)) {
                                            // Send file content
                                            if let Ok(mut file) = File::open(&portal.file_path) {
                                                let mut file_data = Vec::new();
                                                if file.read_to_end(&mut file_data).is_ok() {
                                                    let header = format!(
                                                        "HTTP/1.1 200 OK\r\n\
                                                        Content-Type: application/octet-stream\r\n\
                                                        Content-Disposition: attachment; filename=\"{}\"\r\n\
                                                        Content-Length: {}\r\n\r\n",
                                                        portal.file_info.name, file_data.len()
                                                    );
                                                    let _ = stream.write_all(header.as_bytes());
                                                    let _ = stream.write_all(&file_data);
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                    // Send 404 if file not found
                                    let response = "HTTP/1.1 404 Not Found\r\n\r\nFile not found";
                                    let _ = stream.write_all(response.as_bytes());
                                }
                            } else {
                                // Handle main page request
                                let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                                    <html><body style='font-family:Arial;background:linear-gradient(135deg,#667eea,#764ba2);color:white;padding:20px'>\
                                    <h1>Traverse File Portal</h1>";
                                let _ = stream.write_all(response.as_bytes());
                                
                                if let Ok(portals) = portals_web.lock() {
                                    for portal in portals.values() {
                                        let file_html = format!("<div style='background:rgba(255,255,255,0.1);padding:15px;margin:10px;border-radius:10px'>\
                                            <h3>{}</h3><p>Size: {}</p>\
                                            <a href='/download/{}' style='color:#ffd700;text-decoration:none;background:#28a745;padding:8px 16px;border-radius:5px;display:inline-block;margin-top:10px'>📥 Download</a></div>", 
                                            portal.file_info.name, format_bytes(portal.file_info.size), portal.file_info.hash);
                                        let _ = stream.write_all(file_html.as_bytes());
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
            let addr = format!("192.168.1.{}:{}", i, DISCOVERY_PORT);
            if let Ok(addr) = addr.parse::<SocketAddr>() {
                if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(100)) {
                    let mut buffer = [0; 512];
                    if let Ok(n) = stream.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        if let Some(portal_data) = response.strip_prefix("PORTAL:") {
                            let parts: Vec<&str> = portal_data.trim().split(':').collect();
                            if parts.len() >= 5 {
                                let file_info = FileInfo {
                                    name: parts[1].to_string(),
                                    size: parts[2].parse().unwrap_or(0),
                                    chunk_count: parts[3].parse().unwrap_or(0),
                                    hash: parts[4].to_string(),
                                    room_code: String::new(),
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
        println!("Downloading: {} ({} chunks)", file_info.name.bright_green(), file_info.chunk_count);
        
        let pb = ProgressBar::new(file_info.chunk_count as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks")?
            .progress_chars("=>-"));
        
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
        println!("Saved to: {}", output_path.bright_green());
        Ok(())
    }

    fn join_room(&self, room_code: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Joining room: {}", room_code.bright_yellow());
        
        let url = format!("https://{}/join?room={}", RELAY_SERVER, room_code);
        println!("Connecting to: {}", url.dimmed());
        
        // Try curl first, then PowerShell as fallback
        let output = if let Ok(result) = std::process::Command::new("curl").arg("-s").arg(&url).output() {
            result
        } else {
            let ps_command = format!("Invoke-WebRequest -Uri '{}' -UseBasicParsing | Select-Object -ExpandProperty Content", url);
            std::process::Command::new("powershell").arg("-Command").arg(ps_command).output()
                .unwrap_or_else(|_| std::process::Output { 
                    status: std::process::ExitStatus::default(), 
                    stdout: Vec::new(), 
                    stderr: Vec::new() 
                })
        };
        
        let response = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        if !stderr.is_empty() {
            println!("Debug - stderr: {}", stderr.dimmed());
        }
        
        println!("Debug - Response: {}", response.trim().dimmed());
        
        if response.contains("Joined room") || response.contains("FILE:") {
            println!("{}", "Connected to relay server".bright_green());
            for line in response.lines() {
                if line.starts_with("FILE:") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 4 {
                        println!("{}: {} bytes - Available for download", 
                               parts[1].bright_green(), parts[2]);
                    }
                } else if !line.trim().is_empty() && !line.contains("HTTP/1.1") && !line.starts_with("Joined room") {
                    println!("{}", line.trim());
                }
            }
        } else if response.contains("Room not found") {
            println!("{}: Room {} not found on server", "Connection failed".bright_red(), room_code);
            println!("The room may not be registered yet or the sender is offline");
            println!("Try the local web interface: {}", format!("http://192.168.1.8:8767").bright_cyan());
        } else if response.trim().is_empty() {
            println!("{}: No response from server", "Connection failed".bright_red());
            println!("Check internet connection or try again later");
        } else {
            println!("{}: Unexpected response", "Connection failed".bright_red());
            println!("Server response: {}", response.trim());
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
    use std::process::Command;
    if let Ok(output) = Command::new("ipconfig").output() {
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
        println!("{}", "TRAVERSE v2.0".bright_cyan().bold());
        println!("{}", "Internet P2P File Sharing".dimmed());
        println!("\nUsage:");
        println!("  {} {} <file>     - Share globally", args[0].dimmed(), "send".bright_green());
        println!("  {} {}            - Local discovery", args[0].dimmed(), "recv".bright_blue());
        println!("  {} {} <code>     - Join internet room", args[0].dimmed(), "join".bright_magenta());
        return Ok(());
    }

    let node = TraverseNode::new();

    match args[1].as_str() {
        "send" => {
            if args.len() < 3 { 
                println!("{}", "Error: Specify file to send".bright_red()); 
                return Ok(()); 
            }
            node.send_file(&args[2])?;
            loop { thread::sleep(Duration::from_secs(1)); }
        }
        "join" => {
            if args.len() < 3 { 
                println!("{}", "Error: Specify room code".bright_red()); 
                return Ok(()); 
            }
            node.join_room(&args[2])?;
            loop { thread::sleep(Duration::from_secs(1)); }
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
                println!("{}. {} - {} ({})", 
                    format!("{}", i + 1).bright_yellow(),
                    file_info.name.bright_green(), 
                    format_bytes(file_info.size).bright_blue(), 
                    portal_id.dimmed());
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
        _ => println!("{}", "Unknown command".bright_red()),
    }
    
    Ok(())
}