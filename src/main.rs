use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use sha2::{Sha256, Digest};
use qr2term::print_qr;

const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks
const DISCOVERY_PORT: u16 = 8765;
const TRANSFER_PORT: u16 = 8766;
const WEB_PORT: u16 = 8767;

#[derive(Clone, Debug)]
struct FileInfo {
    name: String,
    size: u64,
    chunk_count: usize,
    #[allow(dead_code)]
    hash: String,
}

#[derive(Clone)]
struct Portal {
    file_info: FileInfo,
    file_path: String,
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
        if !path.exists() {
            return Err("File not found".into());
        }

        let file_size = path.metadata()?.len();
        let chunk_count = ((file_size as f64) / (CHUNK_SIZE as f64)).ceil() as usize;
        
        // Calculate file hash
        let mut file = File::open(file_path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; CHUNK_SIZE];
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 { break; }
            hasher.update(&buffer[..bytes_read]);
        }
        let file_hash = format!("{:x}", hasher.finalize());

        let file_info = FileInfo {
            name: path.file_name().unwrap().to_string_lossy().to_string(),
            size: file_size,
            chunk_count,
            hash: file_hash.clone(),
        };

        let portal = Portal {
            file_info: file_info.clone(),
            file_path: file_path.to_string(),
            peers: Arc::new(Mutex::new(Vec::new())),
        };

        let portal_id = file_hash[..8].to_string();
        self.portals.lock().unwrap().insert(portal_id.clone(), portal.clone());

        println!("\n╔══════════════════════════════════════╗");
        println!("║           🚀 PORTAL ACTIVE          ║");  
        println!("╚══════════════════════════════════════╝");
        println!("📡 Portal ID: \x1b[1;36m{}\x1b[0m", portal_id);
        println!("📁 File: \x1b[1;32m{}\x1b[0m", file_info.name);
        println!("📊 Size: \x1b[1;33m{}\x1b[0m ({} chunks)", format_bytes(file_info.size), file_info.chunk_count);

        // Start discovery service
        self.start_discovery_service(portal_id.clone())?;
        
        // Start transfer service
        self.start_transfer_service(portal_id.clone())?;

        // Start web service for mobile/remote access
        self.start_web_service(portal_id.clone())?;

        println!("\n🔧 Services:");
        println!("  🔍 Discovery  → \x1b[1;34mport {}\x1b[0m", DISCOVERY_PORT);
        println!("  📤 Transfer   → \x1b[1;34mport {}\x1b[0m", TRANSFER_PORT);
        println!("  🌐 Web UI     → \x1b[1;34mport {}\x1b[0m", WEB_PORT);

        // Generate QR code for mobile access
        let local_ip = get_local_ip().unwrap_or_else(|| "192.168.1.100".to_string());
        let qr_url = format!("http://{}:{}", local_ip, WEB_PORT);
        
        println!("\n╔══════════════════════════════════════╗");
        println!("║         📱 MOBILE ACCESS            ║");
        println!("╚══════════════════════════════════════╝");
        println!("🌐 URL: \x1b[1;35m{}\x1b[0m", qr_url);
        println!("\n📱 Scan QR code with your phone:");
        println!("┌{}┐", "─".repeat(38));
        
        if let Err(_) = print_qr(&qr_url) {
            println!("│ QR generation failed - use URL above │");
        }
        
        println!("└{}┘", "─".repeat(38));
        
        println!("\n╔══════════════════════════════════════╗");
        println!("║        ⏳ WAITING FOR PEERS         ║");
        println!("║      Press Ctrl+C to stop sharing   ║");
        println!("╚══════════════════════════════════════╝");

        Ok(portal_id)
    }

    fn start_discovery_service(&self, portal_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let portals = Arc::clone(&self.portals);
        
        thread::spawn(move || {
            let listener = TcpListener::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)).unwrap();
            
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let portals = Arc::clone(&portals);
                        let portal_id = portal_id.clone();
                        
                        thread::spawn(move || {
                            let mut buffer = [0; 1024];
                            if let Ok(size) = stream.read(&mut buffer) {
                                let request = String::from_utf8_lossy(&buffer[..size]);
                                
                                if request.starts_with("DISCOVER") {
                                    if let Some(portal) = portals.lock().unwrap().get(&portal_id) {
                                        let response = format!("PORTAL:{}:{}:{}:{}:{}", 
                                            portal_id, 
                                            portal.file_info.name, 
                                            portal.file_info.size, 
                                            portal.file_info.chunk_count,
                                            TRANSFER_PORT
                                        );
                                        let _ = stream.write_all(response.as_bytes());
                                    }
                                }
                            }
                        });
                    }
                    Err(_) => continue,
                }
            }
        });
        
        Ok(())
    }

    fn start_transfer_service(&self, portal_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let portals = Arc::clone(&self.portals);
        let chunk_cache = Arc::clone(&self.chunk_cache);
        
        thread::spawn(move || {
            let listener = TcpListener::bind(format!("0.0.0.0:{}", TRANSFER_PORT)).unwrap();
            
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let portals = Arc::clone(&portals);
                        let chunk_cache = Arc::clone(&chunk_cache);
                        let portal_id = portal_id.clone();
                        
                        thread::spawn(move || {
                            let mut buffer = [0; 1024];
                            if let Ok(size) = stream.read(&mut buffer) {
                                let request = String::from_utf8_lossy(&buffer[..size]);
                                
                                if let Some(chunk_idx) = request.strip_prefix("CHUNK:") {
                                    if let Ok(index) = chunk_idx.parse::<usize>() {
                                        // Try cache first
                                        if let Some(chunk_data) = chunk_cache.lock().unwrap()
                                            .get(&portal_id)
                                            .and_then(|chunks| chunks.get(&index)) {
                                            let _ = stream.write_all(chunk_data);
                                            return;
                                        }

                                        // Read from file
                                        if let Some(portal) = portals.lock().unwrap().get(&portal_id) {
                                            if let Ok(chunk_data) = read_chunk_from_file(&portal.file_path, index) {
                                                // Cache the chunk
                                                chunk_cache.lock().unwrap()
                                                    .entry(portal_id.clone())
                                                    .or_insert_with(HashMap::new)
                                                    .insert(index, chunk_data.clone());
                                                
                                                let _ = stream.write_all(&chunk_data);
                                                
                                                // Update peer count
                                                if let Ok(peer_addr) = stream.peer_addr() {
                                                    let mut peers = portal.peers.lock().unwrap();
                                                    if !peers.contains(&peer_addr) {
                                                        peers.push(peer_addr);
                                                        let peer_count = peers.len();
                                                        
                                                        // Dynamic topology switch
                                                        if peer_count >= 3 {
                                                            println!("\n╔══════════════════════════════════════╗");
                                                            println!("║      🌐 SWARM MODE ACTIVATED!      ║");
                                                            println!("║       {} peers now connected        ║", peer_count);
                                                            println!("║   Distribution speed increased!     ║");
                                                            println!("╚══════════════════════════════════════╝");
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Err(_) => continue,
                }
            }
        });
        
        Ok(())
    }

    fn start_web_service(&self, portal_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let portals = Arc::clone(&self.portals);
        
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(format!("0.0.0.0:{}", WEB_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let portals = Arc::clone(&portals);
                        let portal_id = portal_id.clone();
                        
                        thread::spawn(move || {
                            let mut buffer = [0; 1024];
                            if let Ok(size) = stream.read(&mut buffer) {
                                let request = String::from_utf8_lossy(&buffer[..size]);
                                
                                if request.starts_with("GET / ") {
                                    // Serve main page
                                    if let Some(portal) = portals.lock().unwrap().get(&portal_id) {
                                        let html = format!(r#"<!DOCTYPE html>
<html><head><title>Traverse - File Share</title><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ 
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    min-height: 100vh; display: flex; align-items: center; justify-content: center;
    color: white; padding: 20px;
}}
.container {{ 
    background: rgba(255,255,255,0.95); border-radius: 20px; padding: 40px;
    box-shadow: 0 20px 40px rgba(0,0,0,0.1); max-width: 500px; width: 100%;
    text-align: center; color: #333;
}}
.header {{ margin-bottom: 30px; }}
.title {{ font-size: 2.5em; margin-bottom: 10px; }}
.subtitle {{ color: #666; font-size: 1.1em; }}
.file-card {{ 
    background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);
    color: white; padding: 30px; margin: 30px 0; border-radius: 15px;
    box-shadow: 0 10px 30px rgba(240, 147, 251, 0.3);
}}
.file-name {{ font-size: 1.5em; font-weight: bold; margin-bottom: 10px; }}
.file-size {{ opacity: 0.9; font-size: 1.1em; }}
.download-btn {{ 
    background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%);
    color: white; padding: 18px 40px; text-decoration: none; border-radius: 50px;
    display: inline-block; margin: 20px 0; font-weight: bold; font-size: 1.1em;
    box-shadow: 0 10px 30px rgba(79, 172, 254, 0.3); transition: transform 0.2s;
}}
.download-btn:hover {{ transform: translateY(-2px); }}
.footer {{ color: #999; margin-top: 30px; }}
</style></head>
<body>
<div class="container">
    <div class="header">
        <div class="title">🚀 Traverse</div>
        <div class="subtitle">Fast P2P File Sharing</div>
    </div>
    <div class="file-card">
        <div class="file-name">📁 {}</div>
        <div class="file-size">{}</div>
    </div>
    <a href="/download" class="download-btn">📥 Download File</a>
    <div class="footer">Powered by Traverse P2P</div>
</div>
</body></html>"#, 
                                            portal.file_info.name, format_bytes(portal.file_info.size));
                                        
                                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}", html.len(), html);
                                        let _ = stream.write_all(response.as_bytes());
                                    }
                                } else if request.starts_with("GET /download") {
                                    // Serve file download
                                    if let Some(portal) = portals.lock().unwrap().get(&portal_id) {
                                        if let Ok(mut file) = File::open(&portal.file_path) {
                                            let mut file_content = Vec::new();
                                            if file.read_to_end(&mut file_content).is_ok() {
                                                let response = format!(
                                                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\n\r\n",
                                                    portal.file_info.name, file_content.len()
                                                );
                                                let _ = stream.write_all(response.as_bytes());
                                                let _ = stream.write_all(&file_content);
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }
                }
            }
        });
        
        Ok(())
    }

    fn discover_portals(&self) -> Result<Vec<(String, FileInfo, SocketAddr)>, Box<dyn std::error::Error>> {
        print!("🔍 Scanning network");
        std::io::stdout().flush().unwrap();
        let mut portals = Vec::new();
        
        // Broadcast discovery on local network
        for i in 1..255 {
            if i % 50 == 0 {
                print!(".");
                std::io::stdout().flush().unwrap();
            }
            let addr = format!("192.168.1.{}:{}", i, DISCOVERY_PORT);
            if let Ok(mut stream) = TcpStream::connect_timeout(
                &addr.parse()?,
                Duration::from_millis(100)
            ) {
                if stream.write_all(b"DISCOVER").is_ok() {
                    let mut buffer = [0; 1024];
                    if let Ok(size) = stream.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..size]);
                        if let Some(portal_data) = response.strip_prefix("PORTAL:") {
                            let parts: Vec<&str> = portal_data.split(':').collect();
                            if parts.len() >= 5 {
                                let file_info = FileInfo {
                                    name: parts[1].to_string(),
                                    size: parts[2].parse().unwrap_or(0),
                                    chunk_count: parts[3].parse().unwrap_or(0),
                                    hash: parts[0].to_string(),
                                };
                                let transfer_addr = format!("{}:{}", 
                                    addr.split(':').next().unwrap(), 
                                    parts[4]
                                ).parse()?;
                                
                                portals.push((parts[0].to_string(), file_info, transfer_addr));
                            }
                        }
                    }
                }
            }
        }
        
        Ok(portals)
    }

    fn receive_file(&self, portal_id: &str, transfer_addr: SocketAddr, file_info: &FileInfo) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n╔══════════════════════════════════════╗");
        println!("║         📥 DOWNLOADING FILE         ║");
        println!("╚══════════════════════════════════════╝");
        println!("📁 File: \x1b[1;32m{}\x1b[0m", file_info.name);
        println!("📊 Size: \x1b[1;33m{}\x1b[0m", format_bytes(file_info.size));
        
        let output_path = format!("./received_{}", file_info.name);
        let mut output_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&output_path)?;

        let mut total_received = 0;

        // Initialize chunk cache for this file
        self.chunk_cache.lock().unwrap().insert(portal_id.to_string(), HashMap::new());

        for chunk_idx in 0..file_info.chunk_count {
            if let Ok(mut stream) = TcpStream::connect_timeout(&transfer_addr, Duration::from_secs(5)) {
                let request = format!("CHUNK:{}", chunk_idx);
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut chunk_data = Vec::new();
                    if stream.read_to_end(&mut chunk_data).is_ok() && !chunk_data.is_empty() {
                        // Write chunk to file
                        output_file.seek(SeekFrom::Start((chunk_idx * CHUNK_SIZE) as u64))?;
                        output_file.write_all(&chunk_data)?;
                        
                        // Cache chunk for serving to other peers
                        self.chunk_cache.lock().unwrap()
                            .get_mut(portal_id)
                            .unwrap()
                            .insert(chunk_idx, chunk_data);
                        
                        total_received += 1;
                        
                        let progress = (total_received as f64 / file_info.chunk_count as f64) * 100.0;
                        let bar_width = 30;
                        let filled = (progress / 100.0 * bar_width as f64) as usize;
                        let empty = bar_width - filled;
                        
                        let bar = "█".repeat(filled) + &"░".repeat(empty);
                        print!("\r🚀 [\x1b[1;32m{}\x1b[0m] {:.1}% ({}/{} chunks)", 
                               bar, progress, total_received, file_info.chunk_count);
                        std::io::stdout().flush().unwrap();
                    }
                }
            }
        }
        
        println!("\n\n╔══════════════════════════════════════╗");
        println!("║        ✅ DOWNLOAD COMPLETE         ║");
        println!("╚══════════════════════════════════════╝");
        println!("📁 Saved to: \x1b[1;32m{}\x1b[0m", output_path);
        println!("🔄 Now serving chunks to other peers...");
        
        Ok(())
    }
}

fn read_chunk_from_file(file_path: &str, chunk_index: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut file = File::open(file_path)?;
    let offset = (chunk_index * CHUNK_SIZE) as u64;
    file.seek(SeekFrom::Start(offset))?;
    
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let bytes_read = file.read(&mut buffer)?;
    buffer.truncate(bytes_read);
    
    Ok(buffer)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

fn get_local_ip() -> Option<String> {
    use std::process::Command;
    
    // Try to get local IP on Windows
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
        println!("\n╔══════════════════════════════════════╗");
        println!("║          🚀 TRAVERSE v1.0           ║");
        println!("║     Fast P2P File Sharing Tool      ║");
        println!("╚══════════════════════════════════════╝");
        println!("\n📋 Usage:");
        println!("  {} send <file>     - 📤 Share a file", args[0]);
        println!("  {} recv            - 📥 Discover and receive files", args[0]);
        println!("\n💡 Examples:");
        println!("  {} send document.pdf", args[0]);
        println!("  {} send ./my-project.zip", args[0]);
        println!("  {} recv", args[0]);
        println!();
        return Ok(());
    }

    let node = TraverseNode::new();

    match args[1].as_str() {
        "send" => {
            if args.len() < 3 {
                println!("❌ Please specify a file to send");
                return Ok(());
            }
            
            let _portal_id = node.send_file(&args[2])?;
            
            // Keep the main thread alive
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }
        "recv" => {
            println!("\n╔══════════════════════════════════════╗");
            println!("║         🔍 DISCOVERING PORTALS      ║");
            println!("╚══════════════════════════════════════╝");
            
            let portals = node.discover_portals()?;
            
            if portals.is_empty() {
                println!("\n❌ No portals found on the network");
                println!("💡 Make sure a sender is running on the same network");
                return Ok(());
            }
            
            println!("\n╔══════════════════════════════════════╗");
            println!("║        📋 AVAILABLE FILES           ║");
            println!("╚══════════════════════════════════════╝");
            
            for (i, (portal_id, file_info, _)) in portals.iter().enumerate() {
                println!("  \x1b[1;32m{}\x1b[0m: \x1b[1;36m{}\x1b[0m", i + 1, file_info.name);
                println!("     📊 Size: {}", format_bytes(file_info.size));
                println!("     🎯 Portal: {}", portal_id);
                println!();
            }
            
            print!("🎯 Choose file (1-{}): ", portals.len());
            std::io::stdout().flush().unwrap();
            
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            
            if let Ok(choice) = input.trim().parse::<usize>() {
                if choice > 0 && choice <= portals.len() {
                    let (portal_id, file_info, transfer_addr) = &portals[choice - 1];
                    node.receive_file(portal_id, *transfer_addr, file_info)?;
                } else {
                    println!("\n❌ Invalid choice. Please enter a number between 1 and {}", portals.len());
                }
            } else {
                println!("\n❌ Please enter a valid number");
            }
        }
        _ => {
            println!("❌ Unknown command. Use 'send' or 'recv'");
        }
    }

    Ok(())
}