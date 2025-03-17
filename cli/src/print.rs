#![allow(dead_code)]

use {
    std::{
        fmt::Display,
        io::Write,
    },
    termcolor::{
        Color,
        ColorChoice,
        ColorSpec,
        StandardStream,
        WriteColor,
    }, unicode_width::UnicodeWidthStr,
};

#[macro_export]
macro_rules! print_status {
    ($label:expr, $emoji:expr, $($args:tt)*) => {
        print_style($label, $emoji, &format!($($args)*), termcolor::Color::Green)
    };
}

#[macro_export]
macro_rules! print_note {
     ($($args: tt)*) => {
        print_style("note", "  ", &format!($($args)*), termcolor::Color::Cyan)
    };
}

#[macro_export]
macro_rules! print_warn {
     ($($args: tt)*) => {
        print_style("warning", "  ", &format!($($args)*), termcolor::Color::Yellow)
    };
}

#[macro_export]
macro_rules! print_error {
     ($($args: tt)*) => {
        print_style("ERROR", "  ", &format!($($args)*), termcolor::Color::Red)
    };
}

/// Print a message with a colored title in the style of Cargo shell messages.
pub fn print_style<S: AsRef<str> + Display>(
    label: S,
    emoji: S,
    message: S,
    color: Color
) {
    let label_str = label.as_ref();
    let emoji_str = emoji.as_ref();
    let message_str = message.as_ref();
    let mut output = StandardStream::stderr(ColorChoice::Auto);

    // Use fixed column positions for better alignment
    const LABEL_COL: usize = 10;  // Adjust as needed
    const EMOJI_COL: usize = 2;   // Adjust as needed

    // Calculate label width and padding
    let label_width = UnicodeWidthStr::width(label_str);
    let label_padding = if label_width < LABEL_COL {
        LABEL_COL - label_width
    } else {
        0
    };
    
    // Print the status with consistent spacing
    output
        .set_color(ColorSpec::new().set_fg(Some(color)).set_bold(true))
        .unwrap();
    
    // Add right padding to the label
    write!(output, "{}{}", " ".repeat(label_padding), label_str).unwrap();
    
    // Add a fixed space for emoji
    write!(output, " {} ", emoji_str).unwrap();
    
    // Reset color and print the message
    output.reset().unwrap();
    writeln!(output, "{}", message_str).unwrap();
}
