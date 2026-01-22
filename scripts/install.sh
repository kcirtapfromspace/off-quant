#!/bin/bash
# Install script for quant CLI
# Installs the quant binary and shell completions

set -e

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
COMPLETIONS_DIR_ZSH="${HOME}/.zsh/completions"
COMPLETIONS_DIR_BASH="${HOME}/.bash_completion.d"

# Get the script's directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY_PATH="${PROJECT_ROOT}/target/release/quant"

echo -e "${BLUE}quant CLI Installer${NC}"
echo "===================="
echo

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo -e "${YELLOW}Release binary not found. Building...${NC}"
    cd "$PROJECT_ROOT"
    cargo build --release --package quant-cli
fi

# Check binary size
BINARY_SIZE=$(du -h "$BINARY_PATH" | cut -f1)
echo -e "Binary size: ${GREEN}${BINARY_SIZE}${NC}"

# Install binary
echo -e "\nInstalling quant to ${BLUE}${INSTALL_DIR}${NC}..."

if [ -w "$INSTALL_DIR" ]; then
    cp "$BINARY_PATH" "$INSTALL_DIR/quant"
    chmod +x "$INSTALL_DIR/quant"
else
    echo -e "${YELLOW}Requires sudo to install to ${INSTALL_DIR}${NC}"
    sudo cp "$BINARY_PATH" "$INSTALL_DIR/quant"
    sudo chmod +x "$INSTALL_DIR/quant"
fi

echo -e "${GREEN}✓${NC} Installed quant to ${INSTALL_DIR}/quant"

# Install shell completions
install_completions() {
    local shell=$1
    local dir=$2
    local file=$3

    mkdir -p "$dir"
    "$INSTALL_DIR/quant" completions "$shell" > "$dir/$file"
    echo -e "${GREEN}✓${NC} Installed $shell completions to $dir/$file"
}

echo -e "\nInstalling shell completions..."

# Detect current shell and install appropriate completions
case "$SHELL" in
    */zsh)
        install_completions zsh "$COMPLETIONS_DIR_ZSH" "_quant"
        echo -e "\n${YELLOW}Note:${NC} Add this to your ~/.zshrc if not already present:"
        echo -e "  fpath=(~/.zsh/completions \$fpath)"
        echo -e "  autoload -Uz compinit && compinit"
        ;;
    */bash)
        install_completions bash "$COMPLETIONS_DIR_BASH" "quant.bash"
        echo -e "\n${YELLOW}Note:${NC} Add this to your ~/.bashrc if not already present:"
        echo -e "  source ~/.bash_completion.d/quant.bash"
        ;;
    *)
        echo -e "${YELLOW}Unknown shell, skipping completions${NC}"
        ;;
esac

# Verify installation
echo
if command -v quant &> /dev/null; then
    VERSION=$(quant --version 2>/dev/null || echo "unknown")
    echo -e "${GREEN}✓ Installation complete!${NC}"
    echo -e "  Version: ${VERSION}"
    echo
    echo "Usage:"
    echo "  quant              # Start interactive chat"
    echo "  quant agent <task> # Run agent with tools"
    echo "  quant --help       # Show all commands"
else
    echo -e "${YELLOW}Warning: quant not found in PATH${NC}"
    echo "You may need to add ${INSTALL_DIR} to your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
