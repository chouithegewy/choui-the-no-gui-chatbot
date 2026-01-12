use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dotenv::dotenv;

use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};

use std::fs;
use tokio::sync::mpsc;
use tui_input::backend::crossterm::EventHandler;

use choui_the_no_gui_chatbot::{
    config::Config,
    state::{App, AppEvent},
    twitch::{
        authenticate_via_device_flow, get_user_id, get_user_login, load_token_cache, refresh_token,
        save_token_cache, send_chat_message, subscribe_to_chat_messages, validate_token,
    },
    ui::ui,
    ws::{connect_eventsub_ws, connect_irc_ws},
};

#[tokio::main]
async fn main() -> Result<()> {
    // env_logger::init(); // Disable logger output to stdout to avoid breaking TUI
    dotenv().ok();

    println!("Twitch EventSub Chat Bot (Rust) starting...");

    let mut config = Config::from_env()?;

    let client = reqwest::Client::new();

    // Authenticate (Device Flow or Cache)
    println!("Authenticating...");
    let token = 'auth: {
        if let Ok(cached) = load_token_cache() {
            println!("Found cached token. Validating...");
            if validate_token(&client, &cached.access_token)
                .await
                .unwrap_or(false)
            {
                println!("Token is valid!");
                break 'auth cached.access_token;
            }

            println!("Token expired or invalid.");
            if let Some(rt) = cached.refresh_token {
                println!("Attempting refresh...");
                if let Ok(new_token) = refresh_token(&client, &config, &rt).await {
                    println!("Refresh successful!");
                    let _ = save_token_cache(&new_token);
                    break 'auth new_token.access_token;
                }
                println!("Refresh failed.");
            }
        }

        println!("Starting Device Authorization Flow...");
        let token_resp = authenticate_via_device_flow(&client, &config).await?;
        token_resp.access_token
    };
    config.oauth_token = Some(token);
    println!("Authentication successful!");

    // Resolve bot_user_id if it's not a numeric ID
    if config.bot_user_id.chars().any(|c| !c.is_numeric()) {
        println!(
            "Detected username for BOT_USER_ID: {}. Resolving to ID...",
            config.bot_user_id
        );
        let id = get_user_id(&client, &config, &config.bot_user_id).await?;
        println!("Resolved Bot ID: {}", id);
        config.bot_user_id = id;
    }

    // Resolve channel_user_id if missing
    if config.channel_user_id.is_none() {
        if let Some(name) = &config.channel_name {
            println!("Resolving ID for channel: {}", name);
            let id = get_user_id(&client, &config, name).await?;
            println!("Resolved ID: {}", id);
            config.channel_user_id = Some(id);
        } else {
            // Panic or error if neither is set, but Config::from_env might handle it differently?
            // Actually Config::from_env just loads vars.
            // Let's rely on logic.
            anyhow::bail!("Neither CHANNEL_USER_ID nor CHANNEL_NAME is set");
        }
    }

    // Ensure we have channel_name (needed for IRC)
    if config.channel_name.is_none() {
        if let Some(id) = &config.channel_user_id {
            println!("Resolving Name for channel ID: {}", id);
            let name = get_user_login(&client, &config, id).await?;
            println!("Resolved Name: {}", name);
            config.channel_name = Some(name);
        }
    }

    println!("Starting UI...");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // --- TUI Setup ---
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config.clone());

    // Use automatic detection for font size, but default to Sixel as requested
    let mut picker = ratatui_image::picker::Picker::from_termios()
        .unwrap_or(ratatui_image::picker::Picker::new((8, 12)));
    picker.protocol_type = ratatui_image::picker::ProtocolType::Sixel;
    app.protocol_name = format!("{:?}", picker.protocol_type);
    app.picker = Some(picker.clone());

    // Fetch Global Emotes (Async in background, but updating state needs care)
    // For simplicity, let's fetch BEFORE event loop or in separate task that sends Event?
    // Let's do it before TUI starts to keep it simple for now, or use channel?
    // Better: Helper function to populate App.
    // Fetching 300 emotes will be SLOW sequentially.
    // Let's try to fetch just the GLOBAL ones first.

    println!("Fetching Global Emotes...");
    let client_clone = client.clone();
    let config_clone = config.clone();

    // We can't easily wait here because TUI isn't up, but we want status.
    // Let's spawn a task to load them and send AppEvent::EmotesLoaded?
    // Implementation:
    // 1. Fetch map
    // 2. Filter map for EMOJIS list (so we only download what we need)
    // 3. Download images
    // 4. Decode
    // 5. Send to main thread

    // Since AppEvent doesn't carry images easily (Protocol is not Send/Sync sometimes?),
    // Actually Protocol IS Send.
    // But `dyn Protocol`...
    // Let's simplify: Just load for "Kappa" and "PogChamp" first to verify.
    // Or try to load all in background.

    // Let's spawn the loader.
    let (tx, mut rx) = mpsc::unbounded_channel();

    let tx_loader = tx.clone();
    tokio::spawn(async move {
        use choui_the_no_gui_chatbot::state::EMOJIS;
        use choui_the_no_gui_chatbot::twitch::{download_emote, get_global_emotes};

        match get_global_emotes(&client_clone, &config_clone).await {
            Ok(map) => {
                let _ = tx_loader.send(AppEvent::Info("Global emote map fetched.".into()));
                // Ensure assets directory exists
                let _ = fs::create_dir_all("assets/emotes");

                for &name in EMOJIS {
                    let file_path = format!("assets/emotes/{}.png", name);
                    let path = std::path::Path::new(&file_path);

                    let img_data = if path.exists() {
                        // Load from file
                        fs::read(path).ok()
                    } else {
                        // Download
                        if let Some(url) = map.get(name) {
                            match download_emote(&client_clone, url).await {
                                Ok(bytes) => {
                                    // Save to file
                                    let _ = fs::write(path, &bytes);
                                    Some(bytes)
                                }
                                Err(_) => None,
                            }
                        } else {
                            None
                        }
                    };

                    if let Some(bytes) = img_data {
                        if let Ok(dyn_img) = image::load_from_memory(&bytes) {
                            let _ = tx_loader.send(AppEvent::EmoteImage(name.to_string(), dyn_img));
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx_loader.send(AppEvent::Error(format!("Failed to fetch emotes: {}", e)));
            }
        }
    });

    // Oops, I can't easily modify AppEvent without another step.
    // Let's MODIFY src/state.rs FIRST to accept Images.
    // Aborting this specific edit to fix state first.

    // Actually, I can use `write_to_file` to update AppEvent?
    // No, I should do it in `src/state.rs`.

    // Let's continue with basic TUI setup but without the loader task fully wired yet.
    // I will return to state.rs.

    let (session_id, _ws_handle) =
        connect_eventsub_ws(client.clone(), config.clone(), tx.clone()).await?;

    // Connect to IRC WebSocket (for Join/Part events)
    let _irc_handle = connect_irc_ws(config.clone(), tx.clone()).await?;

    // Subscribe
    match subscribe_to_chat_messages(&client, &session_id, &config).await {
        Ok(_) => {
            let _ = tx.send(AppEvent::Info("Subscribed to chat".into()));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Error(format!("Subscription failed: {}", e)));
        }
    }

    let mut event_stream = crossterm::event::EventStream::new();

    // Flag to control redraws
    let mut should_render = true;

    loop {
        if should_render {
            terminal.draw(|f| ui(f, &mut app))?;
            should_render = false;
        }

        tokio::select! {
           Some(evt) = rx.recv() => {
               should_render = true;
               match evt {
                   AppEvent::ChatMessage { user, text } => {
                       app.messages.push(format!("{}: {}", user, text));
                   }
                    AppEvent::UserJoined(user) => {
                        app.messages.push(format!("-> {} joined", user));
                        print!("\x07"); // Terminal Bell
                    }
                    AppEvent::UserLeft(user) => {
                        app.messages.push(format!("<- {} left", user));
                    }
                    AppEvent::EmoteImage(name, dyn_img) => {
                        // Create Protocol
                        if let Some(picker) = &mut app.picker {
                             if let Ok(protocol) = picker.new_protocol(dyn_img.clone(), ratatui::layout::Rect::new(0,0,3,2), ratatui_image::Resize::Fit(None)) {
                                 // Store Name, Source Image, and Protocol
                                 app.emote_images.push((name, dyn_img, protocol));
                             }
                        }
                    }
                    AppEvent::Error(msg) => {
                        app.messages.push(format!("Error: {}", msg));
                    }
                    AppEvent::Info(msg) => {
                        app.messages.push(format!("Info: {}", msg));
                    }
               }
           }
           Some(Ok(event)) = event_stream.next() => {
               match event {
                    Event::Key(key) => {
                       should_render = true;
                       if key.kind == event::KeyEventKind::Press {
                           match key.code {
                               KeyCode::Esc => {
                                   app.exit = true;
                               }
                               KeyCode::Char('c') | KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                   app.exit = true;
                               }
                               // Cycle Protocol: Ctrl+P
                               KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                   if let Some(current_picker) = &app.picker {
                                       let current_proto = current_picker.protocol_type;
                                       let next_proto = match current_proto {
                                           ratatui_image::picker::ProtocolType::Halfblocks => ratatui_image::picker::ProtocolType::Sixel,
                                           ratatui_image::picker::ProtocolType::Sixel => ratatui_image::picker::ProtocolType::Kitty,
                                           ratatui_image::picker::ProtocolType::Kitty => ratatui_image::picker::ProtocolType::Iterm2,
                                           ratatui_image::picker::ProtocolType::Iterm2 => ratatui_image::picker::ProtocolType::Halfblocks,
                                       };

                                       // Create new picker
                                       let mut new_picker = ratatui_image::picker::Picker::new((8, 12));
                                       new_picker.protocol_type = next_proto;
                                       app.protocol_name = format!("{:?}", next_proto);
                                       app.picker = Some(new_picker.clone());

                                       // Regenerate all protocols
                                       let mut new_list = Vec::new();
                                       for (name, dyn_img, _) in &app.emote_images {
                                           if let Ok(protocol) = new_picker.new_protocol(dyn_img.clone(), ratatui::layout::Rect::new(0,0,3,2), ratatui_image::Resize::Fit(None)) {
                                               new_list.push((name.clone(), dyn_img.clone(), protocol));
                                           }
                                       }
                                       app.emote_images = new_list;
                                   }
                               }
                               KeyCode::Enter => {
                                   let text: String = app.input.value().into();
                                   if !text.trim().is_empty() {
                                       let config_clone = app.config.clone();
                                       let text_clone = text.clone();
                                       app.messages.push(format!("Me: {}", text));
                                       app.input.reset();

                                       tokio::spawn(async move {
                                          if let Err(_e) = send_chat_message(&text_clone, &config_clone).await {
                                              // log error somehow? -> channel
                                          }
                                       });
                                   }
                               }
                               _ => {
                                   app.input.handle_event(&Event::Key(key));
                               }
                           }
                       }
                    }
                    Event::Mouse(mouse) => {
                       match mouse.kind {
                            event::MouseEventKind::Down(_) | event::MouseEventKind::Up(_) | event::MouseEventKind::Drag(_) | event::MouseEventKind::ScrollDown | event::MouseEventKind::ScrollUp => {
                                should_render = true;
                            }
                            _ => {
                                // Ignore Move events for rendering
                                should_render = false;
                            }
                       }

                       if mouse.kind == event::MouseEventKind::Down(event::MouseButton::Left) {
                            // Check if mouse is within Emoji Chunk using Stored Area
                            let area = app.emote_area;
                            if mouse.column >= area.x && mouse.column < area.x + area.width &&
                               mouse.row >= area.y && mouse.row < area.y + area.height
                            {
                                match mouse.kind {
                                    event::MouseEventKind::ScrollDown => {
                                        should_render = true; // Ensure render

                                        // Calculate max scroll
                                        let img_width = 3 + 1;
                                        // let img_height = 2; // Unused
                                        // Use area.width (which is the Block outer width, or inner? ui.rs stored chunks[1] = outer)
                                        // Wait, ui.rs stored "chunks[1]". That is the OUTER rect.
                                        // So input area needs to account for borders.
                                        let inner_width = area.width.saturating_sub(2) as usize;
                                        let inner_height = area.height.saturating_sub(2) as usize;
                                        let img_height = 2;

                                        let items_per_row = (inner_width / img_width).max(1);
                                        let total_rows = (app.emote_images.len() + items_per_row - 1) / items_per_row;
                                        let visible_rows = inner_height / img_height;

                                        let max_scroll = total_rows.saturating_sub(visible_rows);

                                        if app.emote_scroll < max_scroll {
                                            app.emote_scroll += 1;
                                        }
                                    }
                                    event::MouseEventKind::ScrollUp => {
                                        should_render = true;
                                        app.emote_scroll = app.emote_scroll.saturating_sub(1);
                                    }
                                    event::MouseEventKind::Down(event::MouseButton::Left) => {
                                        should_render = true;

                                        // Scrollbar Interaction Logic
                                        if mouse.column == area.x + area.width - 1 {
                                            // Clicked on the Scrollbar/Right Border
                                            if mouse.row == area.y {
                                                // Top Arrow
                                                app.emote_scroll = app.emote_scroll.saturating_sub(1);
                                            } else if mouse.row == area.y + area.height - 1 {
                                                // Bottom Arrow
                                                // Re-calculate max scroll to be safe
                                                let img_width = 3 + 1;
                                                let inner_width = area.width.saturating_sub(2) as usize;
                                                let inner_height = area.height.saturating_sub(2) as usize;
                                                let img_height = 2; // Fixed height for Sixels

                                                let items_per_row = (inner_width / img_width).max(1);
                                                let total_rows = (app.emote_images.len() + items_per_row - 1) / items_per_row;
                                                let visible_rows = inner_height / img_height;
                                                let max_scroll = total_rows.saturating_sub(visible_rows);

                                                if app.emote_scroll < max_scroll {
                                                    app.emote_scroll += 1;
                                                }
                                            } else {
                                                // Clicked middle of scrollbar?
                                                // Maybe implement Page Up/Down later.
                                                // For now, do nothing, just preventing Emote Click.
                                            }
                                            // RETURN EARLY to prevent falling into grid logic
                                            continue;
                                        }


                                        // Image Grid Logic (Unified for Text too for now, or split?)
                                        if !app.emote_images.is_empty() {
                                            let rel_x = mouse.column.saturating_sub(area.x);
                                            let rel_y = mouse.row.saturating_sub(area.y);
                                            // Debug
                                            // app.messages.push(format!("Click in Emote Area! Rel: {},{}", rel_x, rel_y));

                                            let img_width = 3 + 1; // 3 width + 1 spacing
                                            let img_height = 2; // 2 height

                                            let grid_row = rel_y as usize / img_height as usize;
                                            let grid_col = rel_x as usize / img_width as usize;

                                            let items_per_row = (area.width as usize / img_width as usize).max(1);

                                            let absolute_row = grid_row + app.emote_scroll;
                                            let index = absolute_row * items_per_row + grid_col;

                                            if index < app.emote_images.len() {
                                                let (name, _, _) = &app.emote_images[index];
                                                // app.messages.push(format!("Selected: {}", name));
                                                let new_val = format!("{}{}{} ", app.input.value(), if app.input.value().is_empty() { "" } else { " " }, name);
                                                app.input = app.input.with_value(new_val);
                                            }
                                        } else {
                                           // Text Mode Logic using Area
                                            let rel_x = mouse.column.saturating_sub(area.x);
                                            let rel_y = mouse.row.saturating_sub(area.y);

                                            // Reuse old text logic but with simple relative coords
                                            let width = area.width as usize;
                                            let click_x = rel_x as usize;
                                            let click_y = rel_y as usize;

                                            use choui_the_no_gui_chatbot::state::EMOJIS;
                                            let mut current_x = 0;
                                            let mut current_y = 0;

                                            for emoji in EMOJIS {
                                                 let emoji_len = emoji.chars().count();
                                                 let item_width = emoji_len + 2;

                                                 if current_x + item_width > width {
                                                     current_x = 0;
                                                     current_y += 1;
                                                 }

                                                 if current_y == click_y && click_x >= current_x && click_x < current_x + emoji_len {
                                                     let new_val = format!("{}{}", app.input.value(), emoji);
                                                     app.input = app.input.with_value(new_val);
                                                     break;
                                                 }
                                                 current_x += item_width;
                                            }
                                        }
                                    }
                                   _ => {}
                               }
                           }
                       }
                    }
                    Event::Resize(_, _) => {
                         should_render = true;
                         let _ = terminal.autoresize(); // Ensure backend knows about resize logic if needed
                    }
                    _ => {}
                }
            }
        }

        if app.exit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
