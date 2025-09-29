# Traverse Relay Server 🌐

**Global relay service for Traverse P2P file sharing**

This relay server enables Traverse clients to share files across the internet, not just local networks. Deployed on Render.com for global accessibility.

## 🚀 Live Server

**URL**: `https://traverse-yt17.onrender.com`

The relay server is automatically used by Traverse clients for internet file sharing.

## Features ✨

- 🏠 **Room Management** - 6-digit room codes for file organization
- 📁 **File Registry** - Track available files in each room
- 🌍 **Global Access** - Web interface for any browser/device
- 📤 **File Upload** - Accept file uploads from Traverse clients
- 📥 **File Download** - Serve files via HTTPS to any device
- 🔄 **Auto-registration** - Traverse clients automatically register files

## How It Works 🔧

### Client Registration Flow
1. **Portal Creation**: Traverse client sends file → creates room
2. **Registration**: Client registers with relay server via `/register` endpoint
3. **File Upload**: Client uploads file content to relay server via `/upload`
4. **Global Access**: File becomes available worldwide via web interface

### API Endpoints

#### Registration
```
GET /register?room=123456&portal=abc123&file=myfile.zip&size=1024
```
Registers a file in a room with metadata.

#### Join Room
```
GET /join?room=123456
```
Lists all files available in a room.

#### Upload File
```
POST /upload/123456/abc123
Content-Type: application/octet-stream
[file data]
```
Uploads file content for global access.

#### Download File
```
GET /download/123456/abc123
```
Downloads file with proper headers and filename.

#### Web Interface
```
GET /room/123456
```
Mobile-friendly web page showing all files in room with download buttons.

## Technical Stack 🛠️

- **Runtime**: Pure Rust HTTP server
- **Concurrency**: Thread-based request handling
- **Storage**: In-memory file storage (uploaded files)
- **Deployment**: Render.com with auto-deploy from Git

## Deployment 🚀

### Deploy to Render

1. Fork this repository
2. Connect to Render.com
3. Create new Web Service
4. Set build command: `cd render-server && cargo build --release`
5. Set start command: `./target/release/render-server`
6. Deploy!

### Environment Variables

```bash
PORT=10000  # Default port (auto-set by Render)
```

No other configuration required - the server auto-configures for the deployment environment.

## Usage Examples 💡

### Client Integration
```bash
# Traverse automatically uses relay server
./traverse send myfile.zip
# Room: 123456 | File: myfile.zip
# Internet: https://traverse-yt17.onrender.com/room/123456

# Join from anywhere
./traverse join 123456
```

### Direct Web Access
```bash
# View room contents
curl https://traverse-yt17.onrender.com/join?room=123456

# Download file directly
curl -L -o downloaded.zip https://traverse-yt17.onrender.com/download/123456/abc123
```

### Mobile Web Interface
Open `https://traverse-yt17.onrender.com/room/123456` in any browser for a mobile-friendly interface with download buttons.

## Development 🔧

### Local Development
```bash
cd render-server
cargo run
# Server starts on http://localhost:10000
```

### Testing
```bash
# Register a file
curl "http://localhost:10000/register?room=test&portal=123&file=test.txt&size=100"

# Join room
curl "http://localhost:10000/join?room=test"

# Upload file
curl -X POST -d "test content" "http://localhost:10000/upload/test/123"

# Download file
curl "http://localhost:10000/download/test/123"
```

## Architecture 🏗️

### Request Flow
```
Client → Relay Server → Storage → Web Interface
   ↓         ↓           ↓         ↓
Register → Store Meta → Upload → Serve Files
```

### Storage Design
- **Room Registry**: HashMap of room codes to file lists
- **File Storage**: HashMap of file hashes to binary data
- **Memory-based**: Fast access, automatic cleanup on restart

### Concurrency Model
- **Thread-per-request**: Each HTTP request handled in separate thread
- **Shared State**: Arc<Mutex<>> for thread-safe room and file storage
- **No Blocking**: Non-blocking I/O for file uploads/downloads

## Monitoring 📊

### Health Check
```bash
curl https://traverse-yt17.onrender.com/
# Returns 404 (expected) - server is running
```

### Room Statistics
The server logs all registration and download events for monitoring:
```
Portal abc123 registered file test.txt in room 123456 from IP 1.2.3.4
File abc123 uploaded to room 123456
```

## Security 🔒

### Current Security Model
- **Room Codes**: 6-digit codes provide basic access control
- **No Authentication**: Public relay service (by design)
- **File Integrity**: SHA-256 hashes verify file integrity
- **Temporary Storage**: Files stored temporarily for active sharing

### Future Enhancements
- Private rooms with password protection
- File encryption at rest
- Rate limiting and abuse prevention
- User accounts and file management

## Contributing 🤝

The relay server is designed to be simple and reliable:

- **Minimal Dependencies**: Pure Rust standard library
- **Simple Architecture**: Easy to understand and modify
- **Production Ready**: Handles real-world traffic loads
- **Extensible**: Easy to add new features

## License 📄

MIT License - Same as main Traverse project.

---

**Relay Server | Enabling global P2P file sharing | Built with Rust ❤️**