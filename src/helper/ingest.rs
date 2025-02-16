/// helper for turn a BufRead into a skim stream
use std::io::BufRead;
use std::sync::{Arc, LazyLock};

use crossbeam_channel::Sender;

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
    tx_item: Sender<Vec<Arc<dyn SkimItem>>>,
    opts: SendRawOrBuild,
) {
    let mut frag_buffer = String::with_capacity(128);

    loop {
        // first, read lots of bytes into the buffer
        match source.fill_buf() {
            Ok(bytes_buffer) if bytes_buffer.is_empty() => break,
            Ok(bytes_buffer) => {
                let buffer_len = bytes_buffer.len();

                let string = std::str::from_utf8(bytes_buffer).expect("Could not convert bytes to valid UTF8.");

                let vec = match string.rsplit_once(line_ending as char) {
                    Some((main, frag)) => {
                        let buffer = if !frag_buffer.is_empty() {
                            match main.split_once(line_ending as char) {
                                Some((first, rest)) => {
                                    stitch(&mut frag_buffer, first, line_ending, &opts, &tx_item);
                                    rest
                                }
                                None => {
                                    stitch(&mut frag_buffer, main, line_ending, &opts, &tx_item);
                                    continue;
                                }
                            }
                        } else {
                            main
                        };

                        // we have cleared the frag buffer by this point we can append a new string
                        frag_buffer.push_str(frag);

                        buffer
                            .split(line_ending as char)
                            .map(|line| into_skim_item(line, &opts))
                            .into_iter()
                            .collect()
                    }
                    None if !frag_buffer.is_empty() => {
                        stitch(&mut frag_buffer, string, line_ending, &opts, &tx_item);
                        continue;
                    }
                    None => {
                        vec![into_skim_item(string, &opts)]
                    }
                };

                tx_item
                    .send(vec)
                    .expect("There was an error sending text from the ingest thread to the receiver.");

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

fn stitch(
    old: &mut String,
    new: &str,
    line_ending: u8,
    opts: &SendRawOrBuild,
    tx_item: &Sender<Vec<Arc<dyn SkimItem>>>,
) {
    if !new.starts_with(line_ending as char) {
        old.push_str(new);
        tx_item
            .send(vec![into_skim_item(old, opts)])
            .expect("There was an error sending text from the ingest thread to the receiver.");
        old.clear();
        return;
    }

    let items = [&old, new].iter().map(|line| into_skim_item(line, opts)).collect();

    tx_item
        .send(items)
        .expect("There was an error sending text from the ingest thread to the receiver.");

    old.clear();
}

static EMPTY_STRING: &str = "";
static ARC_EMPTY_STRING: LazyLock<Arc<Box<str>>> = LazyLock::new(|| Arc::new(EMPTY_STRING.into()));

fn into_skim_item(line: &str, opts: &SendRawOrBuild) -> Arc<dyn SkimItem> {
    match opts {
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
        SendRawOrBuild::Raw if line.is_empty() => ARC_EMPTY_STRING.clone(),
        SendRawOrBuild::Raw => {
            let item: Box<str> = line.into();
            Arc::new(item)
        }
    }
}
