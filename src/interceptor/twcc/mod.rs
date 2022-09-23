pub mod capturer;
pub mod handler;

use std::{time::SystemTime, sync::{Arc, Mutex}, collections::BTreeMap};

pub type TwccDataMap = Arc<Mutex<BTreeMap<u32, SystemTime>>>;

const MAX_SEQUENCE_NUMBER_COUNT: usize = 256;
