#![allow(unused)]

use log::{info, warn, error, trace, debug};
use std::fs::File;
use std::io::{Write, BufReader, BufRead};
use failure::{err_msg, Error};
use log4rs::config;
use chrono::{NaiveDateTime, Local};
mod comp;
mod sync;
mod tick;
use comp::*;
use std::{
    i32,
    ops::{Deref, DerefMut},
    sync::{mpsc, Arc},
    time::{Instant, Duration},
    io,thread,
};

use specs::{
    prelude::Resource,
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity as EcsEntity, WorldExt,
};

const TPS: u64 = 10;

fn read_input() -> String {
    let mut buffer = String::new();

    io::stdin()
        .read_line(&mut buffer)
        .expect("Failed to read input");

    buffer.trim().to_string()
}

fn main() -> std::result::Result<(), Error> {
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();
    let mut state = State::new();
    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / TPS as f64));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let msg = read_input();
            tx.send(msg).unwrap();
        }
    });
    loop {
        for msg in rx.try_iter() {
            state.send_chat(msg)
        }
        state.tick(clock.dt());
        
        // Wait for the next tick.
        clock.tick();
    }


    Ok(())
}

pub trait DateTimeNow {
    fn now() -> NaiveDateTime;
}

impl DateTimeNow for NaiveDateTime {
    fn now() -> NaiveDateTime {
        let dt = Local::now();
        dt.naive_local()
    }
}
