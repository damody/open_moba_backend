use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
use failure::Error;
use log::*;
// use math::round;  // 移除problematic math crate
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};
use serde_json::json;


#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayerData {
    pub name: String,
    pub t: String,
    pub a: String,
    pub d: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MqttMsg {
    pub topic: String,
    pub msg: String,
    pub time: SystemTime,
}
impl MqttMsg {
    pub fn new(topic: &String, t: &String, a: &String, v: serde_json::Value) -> MqttMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        };
        let res = ResData {
            t: t.clone(),
            a: a.clone(),
            d: v,
        };
        MqttMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
        }
    }
    pub fn new_s<'a>(topic: &'a str, t: &'a str, a: &'a str, v: serde_json::Value) -> MqttMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        };
        let res = ResData {
            t: t.to_owned(),
            a: a.to_owned(),
            d: v,
        };
        MqttMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
        }
    }
}
impl Default for MqttMsg {
    fn default() -> MqttMsg {
        MqttMsg {
            topic: "".to_owned(),
            msg: "".to_owned(),
            time: SystemTime::now(),
        }
    }
}

use serde_json::ser::Formatter;
pub mod Serializer {
    use super::{io, F32Formatter, Formatter};

    /// Creates a new JSON serializer.
    #[inline]
    pub fn new<W>(writer: W) -> serde_json::ser::Serializer<W, F32Formatter>
    where
        W: io::Write,
    {
        with_formatter(writer, F32Formatter)
    }

    /// Creates a new JSON visitor whose output will be written to the writer
    /// specified.
    #[inline]
    pub fn with_formatter<W, F>(writer: W, formatter: F) -> serde_json::ser::Serializer<W, F>
    where
        W: io::Write,
        F: Formatter,
    {
        serde_json::ser::Serializer::with_formatter(writer, formatter)
    }
}

#[derive(Clone, Debug)]
pub struct F32Formatter;

impl Formatter for F32Formatter {
    #[inline]
    fn write_f32<W: ?Sized>(&mut self, writer: &mut W, value: f32) -> io::Result<()>
    where
        W: io::Write,
    {
        let nearest_int = value.round() as i64;
        if value == (nearest_int as f32) {
            serde_json::ser::CompactFormatter.write_i64(writer, nearest_int)
        } else {
            write!(writer, "{:.3}", value)
        }
    }

    #[inline]
    fn write_f64<W: ?Sized>(&mut self, writer: &mut W, value: f64) -> io::Result<()>
    where
        W: io::Write,
    {
        let nearest_int = value.round() as i64;
        if value == (nearest_int as f64) {
            serde_json::ser::CompactFormatter.write_i64(writer, nearest_int)
        } else {
            write!(writer, "{:.3}", value)
        }
    }
}

impl F32Formatter {
    /// Construct a pretty printer formatter that defaults to using two spaces for indentation.
    pub fn new() -> Self {
        F32Formatter {}
    }
}
