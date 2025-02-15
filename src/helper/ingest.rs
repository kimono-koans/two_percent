/// helper for turn a BufRead into a skim stream
use std::io::BufRead;
use std::sync::{Arc, LazyLock};

use crossbeam_channel::{SendError, Sender};

use crate::field::FieldRange;
use crate::SkimItem;
use regex::Regex;
use std::io::ErrorKind;

use super::item::DefaultSkimItem;

#[derive(Clone)]
pub enum SendRawOrBuild<'a> {
    Raw,
    Build(BuildOptions<'a>),
}

#[derive(Clone)]
pub struct BuildOptions<'a> {
    pub ansi_enabled: bool,
    pub trans_fields: &'a [FieldRange],
    pub matching_fields: &'a [FieldRange],
    pub delimiter: &'a Regex,
}

#[allow(unused_assignments)]
pub fn ingest_loop(
    mut source: Box<dyn BufRead + Send>,
    line_ending: u8,
    tx_item: Sender<Arc<dyn SkimItem>>,
    opts: SendRawOrBuild,
) {
    let mut frag_buffer = String::with_capacity(128);

    loop {
        // first, read lots of bytes into the buffer
        match source.fill_buf() {
            Ok(bytes_buffer) => {
                // break when there is nothing left to read
                if bytes_buffer.is_empty() {
                    break;
                }

                let buffer_len = bytes_buffer.len();

                let string = std::str::from_utf8(bytes_buffer).expect("Could not convert bytes to valid UTF8.");

                match string.rsplit_once(line_ending as char) {
                    Some((main, frag)) => {
                        let mut iter = main.split(line_ending as char);

                        if !frag_buffer.is_empty() {
                            if let Some(first) = iter.next() {
                                frag_buffer.push_str(first);
                                send(&frag_buffer, &opts, &tx_item)
                                    .expect("There was an error sending text from the ingest thread to the receiver.");
                                frag_buffer.clear();
                            }
                        }

                        iter.try_for_each(|line| send(line, &opts, &tx_item))
                            .expect("There was an error sending text from the ingest thread to the receiver.");

                        frag_buffer.push_str(frag);
                    }
                    _ => {
                        if !frag_buffer.is_empty() {
                            frag_buffer.push_str(string);
                            send(&frag_buffer, &opts, &tx_item)
                                .expect("There was an error sending text from the ingest thread to the receiver.");
                            frag_buffer.clear();
                            continue;
                        }

                        send(string.trim(), &opts, &tx_item)
                            .expect("There was an error sending text from the ingest thread to the receiver.");
                    }
                }

                source.consume(buffer_len);
            }
            Err(err) => match err.kind() {
                ErrorKind::Interrupted => continue,
                ErrorKind::UnexpectedEof | _ => {
                    break;
                }
            },
        }
    }
}

static EMPTY_STRING: LazyLock<Arc<Box<str>>> = LazyLock::new(|| {
    let item: Box<str> = "".into();
    Arc::new(item)
});

fn send(
    line: &str,
    opts: &SendRawOrBuild,
    tx_item: &Sender<Arc<dyn SkimItem>>,
) -> Result<(), SendError<Arc<dyn SkimItem>>> {
    let item: Arc<dyn SkimItem> = match opts {
        SendRawOrBuild::Build(opts) => {
            let item = DefaultSkimItem::new(
                line,
                opts.ansi_enabled,
                opts.trans_fields,
                opts.matching_fields,
                opts.delimiter,
            );
            Arc::new(item)
        }
        SendRawOrBuild::Raw if line.is_empty() => EMPTY_STRING.clone(),
        SendRawOrBuild::Raw => {
            let item: Box<str> = line.into();
            Arc::new(item)
        }
    };

    tx_item.send(item)
}
