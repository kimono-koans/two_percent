use crate::field::FieldRange;
use crate::SkimItem;
/// helper for turn a BufRead into a skim stream
use std::io::BufRead;
use std::sync::{Arc, LazyLock};

use crossbeam_channel::Sender;
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
    tx_item: &Sender<Vec<Arc<dyn SkimItem>>>,
    opts: &SendRawOrBuild,
) {
    let mut frag_buffer = String::with_capacity(128);

    'outer: loop {
        // first, read lots of bytes into the buffer
        match source.fill_buf() {
            Ok(bytes_buffer) if bytes_buffer.is_empty() => break,
            Ok(ref mut bytes_buffer) => {
                let buffer_len = bytes_buffer.len();

                'inner: loop {
                    match std::str::from_utf8(bytes_buffer) {
                        Ok(string) => {
                            let _ = process(string, &mut frag_buffer, line_ending, &tx_item, &opts);
                            break 'inner;
                        }
                        Err(err) => {
                            debug!("Removing bytes which are invalid UTF8: {:?}", err);

                            let (valid, after_valid) = bytes_buffer.split_at(err.valid_up_to());
                            let pre_checked = unsafe { std::str::from_utf8_unchecked(valid) };

                            if process(pre_checked, &mut frag_buffer, line_ending, &tx_item, &opts).is_err() {
                                break 'inner;
                            }

                            if let Some(invalid_sequence_length) = err.error_len() {
                                *bytes_buffer = &after_valid[invalid_sequence_length..];
                            }

                            continue 'inner;
                        }
                    };
                }

                source.consume(buffer_len);
            }
            Err(err) => match err.kind() {
                ErrorKind::Interrupted => continue 'outer,
                ErrorKind::UnexpectedEof | _ => {
                    break 'outer;
                }
            },
        }
    }

    if !frag_buffer.is_empty() {
        let items = vec![into_skim_item(&frag_buffer, opts)];
        let _ = send(items, tx_item);
    }
}

fn process(
    base: &str,
    mut frag_buffer: &mut String,
    line_ending: u8,
    tx_item: &Sender<Vec<Arc<dyn SkimItem>>>,
    opts: &SendRawOrBuild,
) -> Result<(), crossbeam_channel::SendError<Vec<Arc<dyn SkimItem>>>> {
    let vec = match base.rsplit_once(line_ending as char) {
        Some((main, frag)) => {
            let buffer = if !frag_buffer.is_empty() {
                match main.split_once(line_ending as char) {
                    Some((first, rest)) => {
                        stitch(&mut frag_buffer, first, line_ending, &opts, &tx_item)?;
                        rest
                    }
                    None => {
                        stitch(&mut frag_buffer, main, line_ending, &opts, &tx_item)?;
                        return Ok(());
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
            stitch(&mut frag_buffer, base, line_ending, &opts, &tx_item)?;
            return Ok(());
        }
        None => {
            vec![into_skim_item(base, &opts)]
        }
    };

    send(vec, tx_item)
}

fn stitch(
    old: &mut String,
    new: &str,
    line_ending: u8,
    opts: &SendRawOrBuild,
    tx_item: &Sender<Vec<Arc<dyn SkimItem>>>,
) -> Result<(), crossbeam_channel::SendError<Vec<Arc<dyn SkimItem>>>> {
    if !new.starts_with(line_ending as char) {
        old.push_str(new);
        let items = vec![into_skim_item(old, opts)];
        send(items, tx_item)?;
        old.clear();
        return Ok(());
    }

    let items = [&old, new].iter().map(|line| into_skim_item(line, opts)).collect();

    send(items, tx_item)?;

    old.clear();

    Ok(())
}

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
        SendRawOrBuild::Raw => {
            let item: Box<str> = line.into();
            Arc::new(item)
        }
    }
}

fn send(
    vec: Vec<Arc<dyn SkimItem>>,
    tx_item: &Sender<Vec<Arc<dyn SkimItem>>>,
) -> Result<(), crossbeam_channel::SendError<Vec<Arc<dyn SkimItem>>>> {
    tx_item.send(vec)
}
