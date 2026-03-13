use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rustdesk-cli")]
#[command(about = "Command-line RustDesk client for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote RustDesk peer
    Connect {
        /// Peer ID to connect to
        peer_id: String,
        /// Password for the peer
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Disconnect from current peer
    Disconnect,
    /// Capture a screenshot from the remote display
    Capture {
        /// Output file path (PNG)
        #[arg(short, long, default_value = "screenshot.png")]
        output: String,
    },
    /// Type text on the remote machine
    Type {
        /// Text to type
        text: String,
    },
    /// Send a key press to the remote machine
    Key {
        /// Key name (e.g. "enter", "tab", "ctrl+c")
        key: String,
    },
    /// Click at coordinates on the remote display
    Click {
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
        /// Mouse button (left, right, middle)
        #[arg(short, long, default_value = "left")]
        button: String,
    },
    /// Move the mouse cursor on the remote display
    Move {
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
    },
    /// Show connection status
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Connect { peer_id, password } => {
            println!("connect: peer_id={peer_id} password={}", password.as_deref().unwrap_or("(none)"));
            eprintln!("not implemented");
        }
        Commands::Disconnect => {
            eprintln!("not implemented");
        }
        Commands::Capture { output } => {
            println!("capture: output={output}");
            eprintln!("not implemented");
        }
        Commands::Type { text } => {
            println!("type: text={text}");
            eprintln!("not implemented");
        }
        Commands::Key { key } => {
            println!("key: {key}");
            eprintln!("not implemented");
        }
        Commands::Click { x, y, button } => {
            println!("click: ({x}, {y}) button={button}");
            eprintln!("not implemented");
        }
        Commands::Move { x, y } => {
            println!("move: ({x}, {y})");
            eprintln!("not implemented");
        }
        Commands::Status => {
            eprintln!("not implemented");
        }
    }

    Ok(())
}
