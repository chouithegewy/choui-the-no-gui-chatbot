use crate::state::App;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(12), // Taller as requested
            Constraint::Length(3),  // Input
        ])
        .split(f.size());

    // Store layout for click detection
    app.emote_area = chunks[1];

    // We need to scroll to the bottom.
    // Ideally use stateful widget, but for now let's just create the list with the right items?
    // Actually, List doesn't auto-scroll. We need to slice the messages or use ListState.
    // Let's rely on re-slicing for simplicity as per plan.
    let chat_area_height = chunks[0].height.saturating_sub(2) as usize; // Subtract 2 for borders

    let start_index = app.messages.len().saturating_sub(chat_area_height);

    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .skip(start_index)
        .map(|m| ListItem::new(Line::from(vec![Span::raw(m)])))
        .collect();

    let messages_list =
        List::new(messages).block(Block::default().borders(Borders::ALL).title("Chat"));
    f.render_widget(messages_list, chunks[0]);

    // Emoji Bar
    // We render images if available, otherwise fallback to text?
    // Mixed rendering is hard.
    // If we have images, we should render them in a grid.

    // Check if we have loaded ANY images
    if !app.emote_images.is_empty() {
        // Render Sixel Images
        let outer_block = Block::default().borders(Borders::ALL).title(format!(
            "Emotes (Click) [{}] ({})",
            app.emote_images.len(),
            app.protocol_name
        ));

        let inner_area = outer_block.inner(chunks[1]);
        f.render_widget(outer_block, chunks[1]);

        // Grid Logic
        // Natural size: 3x2 (approx 28x28px)
        let img_width: u16 = 3;
        let img_height: u16 = 2;

        let inner_width = inner_area.width;
        let inner_height = inner_area.height;
        let items_per_row = (inner_width / (img_width + 1)).max(1);

        let total_items = app.emote_images.len();
        let total_rows = (total_items + items_per_row as usize - 1) / items_per_row as usize;

        // Render images based on scroll
        let start_row = app.emote_scroll;
        let end_row = start_row + (inner_height / img_height) as usize + 1;

        let start_index = start_row * items_per_row as usize;

        let mut x_offset = inner_area.x;
        let mut y_offset = inner_area.y;

        for (i, (_name, _dyn_img, protocol)) in
            app.emote_images.iter().enumerate().skip(start_index)
        {
            let row_idx = i / items_per_row as usize;
            if row_idx >= end_row {
                break;
            }

            // Calculate position relative to inner_area
            let visible_row = row_idx - start_row;

            let col_idx = i % items_per_row as usize;

            x_offset = inner_area.x + (col_idx as u16 * (img_width + 1));
            y_offset = inner_area.y + (visible_row as u16 * img_height);

            if y_offset + img_height > inner_area.bottom() {
                break;
            }

            let area = ratatui::layout::Rect::new(x_offset, y_offset, img_width, img_height);

            let image_widget = ratatui_image::Image::new(protocol.as_ref());
            f.render_widget(image_widget, area);
        }

        // Render Scrollbar
        let scrollbar = ratatui::widgets::Scrollbar::default()
            .orientation(ratatui::widgets::ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let height_in_rows = inner_height as usize / img_height as usize;
        // let max_scroll = total_rows.saturating_sub(height_in_rows);

        // User requested inverted visual? "Switch this".
        // If they see it "upside down" currently (Scroll 0 = Top, Scroll Max = Bottom),
        // and they want it switched?
        // Actually, let's stick to standard first: Top = 0.
        // But user says "click down arrow, bar is at top". This implies Scroll increments, but Bar is visually at top.
        // This usually implies logical error.
        // Let's try to ensure we use the robust Total/Viewport API.

        let mut scrollbar_state = ratatui::widgets::ScrollbarState::new(total_rows)
            .viewport_content_length(height_in_rows)
            .position(app.emote_scroll);
        f.render_stateful_widget(
            scrollbar,
            chunks[1], // Render over the block
            &mut scrollbar_state,
        );
    } else {
        // Fallback to text
        let emoji_text = crate::state::EMOJIS.join("  ");
        let emojis = Paragraph::new(emoji_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Emotes (Loading...)"),
            )
            .style(Style::default().fg(Color::Cyan))
            .wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(emojis, chunks[1]);
    }

    let input = Paragraph::new(app.input.value())
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[2]);

    // Cursor
    f.set_cursor(
        chunks[2].x + app.input.visual_cursor() as u16 + 1,
        chunks[2].y + 1,
    );
}
