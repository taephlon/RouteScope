# RouteScope 🌐🔭

**RouteScope** is a modern, high-performance network diagnostics tool written in Rust — combining the capabilities of **MTR**, **GeoIP lookups**, **AS Path visualization**, **real-time latency profiling**, and a beautiful **Web Dashboard** with an interactive map.

---

## ✨ Features

| Feature | Description |
|---|---|
| 🔀 **Multi-Protocol** | ICMP, UDP (default, unprivileged), and TCP traceroute |
| 📡 **MTR-Style Continuous Mode** | Live TUI that continuously probes all hops |
| 🔓 **Unprivileged Mode** | Runs without `sudo` using Linux `MSG_ERRQUEUE` + `IP_RECVERR` |
| 🗺 **GeoIP & ASN** | Country, City, Lat/Lon, ASN, ISP, Timezone per hop |
| 💾 **Smart Cache** | Disk-based GeoIP cache (`.routescope_geoip_cache.json`) for instant repeats |
| 📈 **Rich Statistics** | RTT (last/avg/min/max), Jitter, Packet Loss % per hop |
| 🌡 **Latency Heatmap** | ASCII block heatmap showing where latency spikes |
| 📉 **Latency Chart** | ASCII scatter plot of per-hop RTT |
| 🔄 **Reverse DNS** | Hostname resolution for every responding hop |
| 📤 **Export** | JSON, CSV, GeoJSON (compatible with QGIS, Google Earth, Leaflet, MapLibre) |
| 🖥 **Interactive TUI** | `ratatui`-based live table with sparkline, geo panel, loss gauge |
| 🌐 **Web Dashboard** | Embedded SPA with Leaflet map, Chart.js graphs, real-time WebSocket updates |
| 🌍 **IPv4 & IPv6** | Full dual-stack support |

---

## 📁 Project Layout

```
routescope/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI parser & mode router
│   ├── traceroute.rs     # Core MTR-style orchestrator loop
│   ├── probe.rs          # Network sockets (UDP/ICMP/TCP probes)
│   ├── geoip.rs          # GeoIP client + disk cache
│   ├── dns.rs            # Async DNS resolver + reverse DNS
│   ├── stats.rs          # RTT, Loss, Jitter calculators
│   ├── export.rs         # JSON, CSV, GeoJSON writers
│   ├── tui.rs            # ratatui + crossterm live TUI
│   ├── map.rs            # ASCII map, heatmap, latency chart
│   ├── web_dashboard.rs  # Axum + WebSocket server
│   └── dashboard.html    # Self-contained web SPA
└── geoip/                # Optional: place MaxMind .mmdb files here
    ├── GeoLite2-City.mmdb
    └── GeoLite2-ASN.mmdb
```

---

## 🚀 Installation & Building

To compile and install RouteScope on any Linux distribution, you can use either the automated installation script, the Makefile, or standard Cargo.

### Option 1: Automated Shell Script (Recommended)
This script builds RouteScope and automatically installs it to `/usr/local/bin/` (or falls back to `~/.local/bin/` if run without root/sudo privileges). It also automatically attempts to grant raw socket capabilities so you don't need `sudo` for ICMP/TCP modes:
```bash
./install.sh
```

### Option 2: Using Makefile
If you prefer standard system tools, a `Makefile` is provided:
```bash
# Build only
make

# Install system-wide
sudo make install

# Uninstall
sudo make uninstall
```

### Option 3: Manual Installation (Cargo)
```bash
# Build (optimized release binary)
cargo build --release

# Install globally to ~/.cargo/bin/ (which must be in your $PATH)
cargo install --path .

# (Optional) Grant raw socket capability to the binary
sudo setcap cap_net_raw+ep target/release/routescope
```

---

## 📖 Usage

Once installed, you can run the `routescope` command from anywhere. If you did not install it, you can run the compiled binary directly from the build directory: `./target/release/routescope`.

### Interactive Live TUI (MTR Mode) — default
```bash
# Basic trace (uses unprivileged UDP by default - no sudo/installation required!)
routescope google.com

# Using ICMP Echo requests (requires cap_net_raw or sudo)
routescope 8.8.8.8 --icmp

# Using TCP SYN probes on port 80
routescope github.com --tcp

# Specify 100 probe rounds
routescope 1.1.1.1 -c 100

# Force IPv4 or IPv6 resolution
routescope google.com -4
routescope google.com -6
```

**TUI Keybindings:**
| Key | Action |
|---|---|
| `↑` / `k` | Select previous hop |
| `↓` / `j` | Select next hop |
| `Space` | Pause/resume live updates |
| `q` / `Esc` | Quit |

### Export Mode (non-interactive)
```bash
# JSON export
routescope github.com --json trace.json

# CSV export
routescope 8.8.8.8 --csv trace.csv

# GeoJSON export (open in QGIS, Google Earth, Leaflet, MapLibre)
routescope google.com --geojson trace.geojson

# ASCII map + latency chart + heatmap + summary table
routescope google.com --map

# All at once
routescope github.com --json out.json --csv out.csv --geojson out.geojson --map
```

### Web Dashboard
```bash
# Start the web server (browser opens at http://localhost:8080)
routescope --web

# Custom port
routescope --web --web-port 9090
```

Then open **http://localhost:8080** in your browser.

---

## 🌍 GeoIP Data Sources

RouteScope uses a two-tier approach:

1. **Local MaxMind databases** (fast, offline): Place `GeoLite2-City.mmdb` and `GeoLite2-ASN.mmdb` in the `geoip/` directory. Download from [MaxMind](https://dev.maxmind.com/geoip/geolite2-free-geolocation-data) (free registration required).
2. **freeipapi.com** (automatic fallback): No configuration needed, results are cached in `.routescope_geoip_cache.json`.

---

## 🎨 Web Dashboard Features

- **Dark glassmorphism** UI with `Outfit` + `JetBrains Mono` typography
- **Interactive Leaflet map** with pulsing markers for each hop
- **Real-time Chart.js** line & bar graphs (RTT + Jitter)
- **Live hop metrics table** with color-coded latency pills
- **AS Transit Flow** sidebar showing provider changes
- **Export buttons**: JSON, CSV, GeoJSON directly from the browser
- **WebSocket-driven** — updates every probe without page refresh

---

## 📊 Example Output (CLI)

```
$ routescope github.com --map

╔══════════════════════════════════════╗
║   RouteScope — Geographic Path       ║
╚══════════════════════════════════════╝
  📍 Indonesia
      │
      │
  📍 Singapore  [AS7473]
      │
      │
  📍 United States  [AS36459]

╔══════════════════════════════════════╗
║   RouteScope — Latency Heatmap       ║
╚══════════════════════════════════════╝
  Hop  1      0.30 ms  █
  Hop  2      1.20 ms  ██
  Hop  3      3.80 ms  ██████
  Hop  4     12.10 ms  ████████████████████
  Hop  5    172.00 ms  ████████████████████████████████████████

Hop  IP Address           Hostname                  Country/City   ASN        Avg RTT   Loss%  RTT Heat
───────────────────────────────────────────────────────────────────────────────────────────────────────
1    192.168.1.1          router.local              Local Network  N/A        0.30 ms   0.0%  █
2    10.30.0.1            gw.isp.net                Indonesia      AS7713     1.20 ms   0.0%  ██
3    103.x.x.x            ae5.sng.isp.net           Singapore      AS7473    12.10 ms   0.0%  ████████████████████
4    52.95.x.x            ae9.amazon.com            United States  AS16509  172.00 ms   0.0%  ████████████████████████████████████████
```

---

## 🔧 Advanced & CLI Examples

Once installed, you can use these flags in any order:

### Run with more probe rounds (MTR-style)
```bash
routescope google.com -c 500
```

### Set custom timeout (in milliseconds)
```bash
routescope 8.8.8.8 --timeout 2000
```

### Limit to N hops
```bash
routescope google.com -m 15
```

### Use a custom destination port (TCP mode)
```bash
routescope google.com --tcp -p 443
```

### Optional Route Performance Analysis
If you want to view a detailed performance summary of the fastest, slowest, and unreliable hops on completion:
```bash
routescope google.com --analyze
# OR
routescope google.com -a
```

---

## 💡 Troubleshooting & Tips

### 1. Rootless ICMP (Ping) setup
If you do not want to use `setcap` or `sudo` to run ICMP traces, you can configure your Linux system's group ranges to allow user-space ping sockets.
Run this command to allow all users on the system to open ping sockets:
```bash
sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"
```
To make this setting permanent across reboots, add it to your sysctl config:
```bash
echo 'net.ipv4.ping_group_range = 0 2147483647' | sudo tee -a /etc/sysctl.conf
```

### 2. Running behind Remote IDEs, Codespaces, or Proxies
If the **Web Dashboard** loads in your browser but the WebSocket connection status remains "Disconnected":
* **VS Code / Github Codespaces Port Forwarding:** By default, port forwarders might block WebSockets. Right-click on port `8080` in the **Ports** panel, select **Port Protocol**, and ensure it matches the protocol you are using (HTTP/HTTPS). Also, try setting the port visibility to **Public**.
* **Nginx Reverse Proxy:** If proxying RouteScope behind Nginx, you must explicitly enable WebSockets by forwarding the upgrade headers:
  ```nginx
  location /api/trace/ws {
      proxy_pass http://localhost:8080/api/trace/ws;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "Upgrade";
      proxy_set_header Host $host;
  }
  ```

### 3. Improving GeoIP lookup speeds
* Out-of-the-box, RouteScope uses a fast online API (`freeipapi.com`) and caches results inside `.routescope_geoip_cache.json` for subsequent traceroutes.
* For **instant, offline** GeoIP resolution, download the free `GeoLite2-City` and `GeoLite2-ASN` databases in `.mmdb` format from MaxMind, and place them inside a folder named `geoip/` in your workspace directory. RouteScope will automatically detect them and switch to offline-only mode.

---

## 🧑‍💻 Developer Community Guide

Contributions from the developer community are highly welcome! Here is an overview of how the codebase is structured and how you can work on it.

### Codebase Architecture Map

* `src/main.rs`: Entry point. Parses CLI arguments using `clap` and handles routing between TUI, Web, and Export modes.
* `src/probe.rs`: Low-level network interaction. Sets up sockets, enables kernel options (like `IP_RECVERR`), constructs packets, and handles unsafe C bindings for querying the error queue.
* `src/traceroute.rs`: The loop orchestrator. Spawns tasks to probe hops sequentially, handles reverse DNS lookups, queries GeoIP, and sends status updates down a channel.
* `src/web_dashboard.rs`: Spawns the Axum-based web server and manages WebSocket upgrade connections.
* `src/dashboard.html`: The frontend SPA. Displays Leaflet maps, Chart.js plots, and provides interactive table sorting.
* `src/stats.rs`: Computes jitter, packet loss percentage, and RTT averages.
* `src/tui.rs` & `src/map.rs`: Interactive and non-interactive ASCII renderer code.

### Build and Development Commands

```bash
# Clone the repository
git clone https://github.com/your-username/routescope.git
cd routescope

# Run tests
cargo test

# Check formatting
cargo fmt -- --check

# Run lints
cargo clippy -- -D warnings

# Build debug binary
cargo build

# Build optimized production binary
cargo build --release
```

### Socket Implementation Notes
When writing code for `src/probe.rs`, please keep in mind:
* **Unprivileged UDP:** Standard `SOCK_DGRAM` sockets are used. ICMP errors are pulled using `recvmsg(fd, ..., MSG_ERRQUEUE)`.
* **Platform Support:** The error queue options (`IP_RECVERR` and `MSG_ERRQUEUE`) are Linux-specific. For other OS platforms (macOS/Windows), fallback raw socket logic is required. All unsafe pointer operations for control messages (`cmsghdr`) must align properly to prevent undefined behavior.
