# Traverse Relay Server

Railway deployment for Traverse P2P file sharing.

## Deploy to Railway

1. Connect this repository to Railway
2. Set port to 80
3. Deploy!

The relay server handles:
- Room creation and management
- File listings across internet
- Cross-device connectivity

## Environment Variables

None required - runs on port 80 by default.

## Usage

Client connects to relay server to:
- Register files in rooms
- Join rooms to see available files
- Download files via HTTPS links