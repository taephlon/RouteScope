#!/usr/bin/env bash
set -e

# Colors for pretty printing
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== RouteScope Installer ===${NC}"

# Function to detect package manager and install build dependencies
install_system_deps() {
    echo -e "${BLUE}Checking system packages (compiler and libcap)...${NC}"
    
    # Identify missing items
    MISSING_CC=0
    MISSING_SETCAP=0
    
    if ! command -v cc &> /dev/null && ! command -v gcc &> /dev/null && ! command -v clang &> /dev/null; then
        MISSING_CC=1
    fi
    if ! command -v setcap &> /dev/null; then
        MISSING_SETCAP=1
    fi
    
    if [ $MISSING_CC -eq 0 ] && [ $MISSING_SETCAP -eq 0 ]; then
        echo -e "${GREEN}System package dependencies are already satisfied.${NC}"
        return 0
    fi
    
    # Detect package manager
    if command -v apt-get &> /dev/null; then
        echo -e "${YELLOW}Detected Debian/Ubuntu-based system.${NC}"
        if [ $MISSING_CC -eq 1 ]; then
            echo -e "${BLUE}Installing build-essential...${NC}"
            sudo apt-get update && sudo apt-get install -y build-essential
        fi
        if [ $MISSING_SETCAP -eq 1 ]; then
            echo -e "${BLUE}Installing libcap2-bin (for setcap)...${NC}"
            sudo apt-get update && sudo apt-get install -y libcap2-bin
        fi
    elif command -v dnf &> /dev/null; then
        echo -e "${YELLOW}Detected Fedora/RHEL-based system.${NC}"
        if [ $MISSING_CC -eq 1 ]; then
            echo -e "${BLUE}Installing Development Tools...${NC}"
            sudo dnf groupinstall -y "Development Tools"
        fi
        if [ $MISSING_SETCAP -eq 1 ]; then
            echo -e "${BLUE}Installing libcap...${NC}"
            sudo dnf install -y libcap
        fi
    elif command -v pacman &> /dev/null; then
        echo -e "${YELLOW}Detected Arch Linux system.${NC}"
        if [ $MISSING_CC -eq 1 ]; then
            echo -e "${BLUE}Installing base-devel...${NC}"
            sudo pacman -Sy --noconfirm base-devel
        fi
        if [ $MISSING_SETCAP -eq 1 ]; then
            echo -e "${BLUE}Installing libcap...${NC}"
            sudo pacman -Sy --noconfirm libcap
        fi
    elif command -v yum &> /dev/null; then
        echo -e "${YELLOW}Detected CentOS/RHEL-based system.${NC}"
        if [ $MISSING_CC -eq 1 ]; then
            echo -e "${BLUE}Installing Development Tools...${NC}"
            sudo yum groupinstall -y "Development Tools"
        fi
        if [ $MISSING_SETCAP -eq 1 ]; then
            echo -e "${BLUE}Installing libcap...${NC}"
            sudo yum install -y libcap
        fi
    else
        echo -e "${YELLOW}Warning: Unknown package manager. Please ensure a C compiler and 'setcap' are installed manually.${NC}"
    fi
}

# 1. Install system tools/compilers
install_system_deps

# 2. Check if cargo is installed, offer to install Rust if missing
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}Rust / Cargo is not installed.${NC}"
    read -p "Would you like to automatically download and install Rust? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${BLUE}Downloading and installing Rust...${NC}"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # Load cargo path into current shell environment
        source "$HOME/.cargo/env"
    else
        echo -e "${RED}Error: Cannot compile RouteScope without Cargo. Exiting.${NC}"
        exit 1
    fi
fi

# Double check that cargo is now available
if ! command -v cargo &> /dev/null; then
    # Fallback path checking
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    else
        echo -e "${RED}Error: Cargo was installed but couldn't be loaded into this shell session. Please restart your terminal and run the installer again.${NC}"
        exit 1
    fi
fi

# 3. Build the release binary
echo -e "${BLUE}Building RouteScope in release mode...${NC}"
cargo build --release

# 4. Determine installation path
INSTALL_DIR="/usr/local/bin"

if [ "$EUID" -ne 0 ]; then
    # Check if we can run sudo
    if command -v sudo &> /dev/null && sudo -n true 2>/dev/null; then
        echo -e "${BLUE}Installing to $INSTALL_DIR (using sudo)...${NC}"
        sudo cp target/release/routescope "$INSTALL_DIR/"
        
        # Try to grant raw socket capabilities
        if command -v setcap &> /dev/null; then
            echo -e "${BLUE}Granting raw socket capabilities (allows ICMP/TCP without root)...${NC}"
            sudo setcap cap_net_raw+ep "$INSTALL_DIR/routescope" || echo -e "${RED}Warning: Failed to set cap_net_raw. You may need sudo to run ICMP/TCP modes.${NC}"
        fi
    else
        # Install to user local bin if no sudo/root privileges
        INSTALL_DIR="$HOME/.local/bin"
        echo -e "${BLUE}No root/sudo privileges. Installing to user local bin: $INSTALL_DIR${NC}"
        mkdir -p "$INSTALL_DIR"
        cp target/release/routescope "$INSTALL_DIR/"
        echo -e "${RED}Note: Cannot setcap cap_net_raw without root. ICMP/TCP modes might require running with sudo.${NC}"
    fi
else
    # We are root
    echo -e "${BLUE}Installing to $INSTALL_DIR...${NC}"
    cp target/release/routescope "$INSTALL_DIR/"
    
    # Try to grant raw socket capabilities
    if command -v setcap &> /dev/null; then
        echo -e "${BLUE}Granting raw socket capabilities (allows ICMP/TCP without root)...${NC}"
        setcap cap_net_raw+ep "$INSTALL_DIR/routescope" || echo -e "${RED}Warning: Failed to set cap_net_raw.${NC}"
    fi
fi

echo -e "${GREEN}RouteScope installed successfully to $INSTALL_DIR/routescope${NC}"
echo "You can now run it by typing: routescope <target>"
