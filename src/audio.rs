use std::fs::File;
use std::io::BufReader;
use std::thread;

pub fn play_sound(path: String) {
    thread::spawn(move || {
        // rodio requires the OutputStream to stay alive while playing.
        // It plays on the thread it's created on (mostly), or at least the stream handle must be kept.
        let (_stream, stream_handle) = match rodio::OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Audio Error: Failed to get output stream: {}", e);
                return;
            }
        };

        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Audio Error: Failed to open file '{}': {}", path, e);
                return;
            }
        };

        let source = match rodio::Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Audio Error: Failed to decode audio: {}", e);
                return;
            }
        };

        if let Err(e) = stream_handle.play_raw(rodio::Source::convert_samples(source)) {
            eprintln!("Audio Error: Failed to play sound: {}", e);
            return;
        }

        // Keep thread alive until finished?
        // rodio's play_raw is non-blocking on the handle, but we need to keep _stream alive.
        // A simple way is to sleep, or use Sink which controls playback better.
        // Let's use Sink.

        let sink = match rodio::Sink::try_new(&stream_handle) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Audio Error: Failed to create sink: {}", e);
                return;
            }
        };

        // Re-open file/decode for Sink (simpler)
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => return, // Already logged
        };
        let source = match rodio::Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(_) => return,
        };

        sink.append(source);
        sink.sleep_until_end();
    });
}
