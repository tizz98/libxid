//! (This is a port of [Olivier Poitrey]'s [xid] Go library)
//!
//! Package xid is a globally unique id generator library, ready to be used safely directly in your server code.
//!
//! Xid is using Mongo Object ID algorithm to generate globally unique ids with a different serialization (base64) to make it shorter when transported as a string:
//! https://docs.mongodb.org/manual/reference/object-id/
//!
//! - 4-byte value representing the seconds since the Unix epoch,
//! - 3-byte machine identifier,
//! - 2-byte process id, and
//! - 3-byte counter, starting with a random value.
//!
//! The binary representation of the id is compatible with Mongo 12 bytes Object IDs.
//! The string representation is using base32 hex (w/o padding) for better space efficiency
//! when stored in that form (20 bytes). The hex variant of base32 is used to retain the
//! sortable property of the id.
//!
//! Xid doesn't use base64 because case sensitivity and the 2 non alphanum chars may be an
//! issue when transported as a string between various systems. Base36 wasn't retained either
//! because 1/ it's not standard 2/ the resulting size is not predictable (not bit aligned)
//! and 3/ it would not remain sortable. To validate a base32 `xid`, expect a 20 chars long,
//! all lowercase sequence of `a` to `v` letters and `0` to `9` numbers (`[0-9a-v]{20}`).
//!
//! UUIDs are 16 bytes (128 bits) and 36 chars as string representation. Twitter Snowflake
//! ids are 8 bytes (64 bits) but require machine/data-center configuration and/or central
//! generator servers. xid stands in between with 12 bytes (96 bits) and a more compact
//! URL-safe string representation (20 chars). No configuration or central generator server
//! is required so it can be used directly in server's code.
//!
//! | Name        | Binary Size | String Size    | Features
//! |-------------|-------------|----------------|----------------
//! | [UUID]      | 16 bytes    | 36 chars       | configuration free, not sortable
//! | [shortuuid] | 16 bytes    | 22 chars       | configuration free, not sortable
//! | [Snowflake] | 8 bytes     | up to 20 chars | needs machin/DC configuration, needs central server, sortable
//! | [MongoID]   | 12 bytes    | 24 chars       | configuration free, sortable
//! | xid         | 12 bytes    | 20 chars       | configuration free, sortable
//!
//! [UUID]: https://en.wikipedia.org/wiki/Universally_unique_identifier
//! [shortuuid]: https://github.com/stochastic-technologies/shortuuid
//! [Snowflake]: https://blog.twitter.com/2010/announcing-snowflake
//! [MongoID]: https://docs.mongodb.org/manual/reference/object-id/
//!
//! Features:
//!
//! - Size: 12 bytes (96 bits), smaller than UUID, larger than snowflake
//! - Base32 hex encoded by default (20 chars when transported as printable string, still sortable)
//! - Non configured, you don't need set a unique machine and/or data center id
//! - K-ordered
//! - Embedded time with 1 second precision
//! - Unicity guaranteed for 16,777,216 (24 bits) unique ids per second and per host/process
//! - Lock-free (i.e.: unlike UUIDv1 and v2)
//!
//! Notes:
//!
//! - Xid is dependent on the system time, a monotonic counter and so is not cryptographically secure.
//! If unpredictability of IDs is important, you should NOT use xids.
//! It is worth noting that most of the other UUID like implementations are also not cryptographically secure.
//! You shoud use libraries that rely on cryptographically secure sources if you want a truly random ID generator.
//!
//! References:
//!
//! - https://www.slideshare.net/davegardnerisme/unique-id-generation-in-distributed-systems
//! - https://en.wikipedia.org/wiki/Universally_unique_identifier
//! - https://blog.twitter.com/2010/announcing-snowflake
//!
//! ## Usage
//!
//! ```rust
//! use libxid;
//!
//! // initialize it once, reuse it afterwards
//! let mut g = libxid::new_generator();
//!
//! for i in 0..10{
//!     let id = g.new_id().unwrap();
//!
//!     println!(
//!             "encoded: {:?}    machine: {:?}    counter: {:?}    time: {:?}",
//!             id.encode(),
//!             id.machine(),
//!             id.counter(),
//!             id.time()
//!     );
//! }
//! ```
//!
//! [Olivier Poitrey]: https://github.com/rs
//! [xid]: https://github.com/rs/xid

extern crate byteorder;
extern crate crc32fast;
extern crate data_encoding;
extern crate gethostname;
extern crate md5;
extern crate rand;

use byteorder::{BigEndian, ByteOrder};
use crc32fast::Hasher;
use data_encoding::{Encoding, Specification, SpecificationError};
use gethostname::*;
use rand::prelude::*;
use std::fmt;
use std::fs;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

const ID_LEN: usize = 12;

pub struct ID {
    val: [u8; ID_LEN],
}

pub struct Generator {
    counter: AtomicUsize,
    machine_id: [u8; 3],
    pid: u32,
}

pub fn new_generator() -> Generator {
    return Generator {
        counter: rand_int(),
        machine_id: read_machine_id(),
        pid: get_pid(),
    };
}

impl Generator {
    pub fn new_id(&mut self) -> Result<ID, SystemTimeError> {
        self.new_id_with_time(SystemTime::now())
    }

    pub fn new_id_with_time(&mut self, t: SystemTime) -> Result<ID, SystemTimeError> {
        match t.duration_since(UNIX_EPOCH) {
            Ok(n) => Ok(self.generate(n.as_secs())),
            Err(e) => Err(e),
        }
    }

    fn generate(&self, ts: u64) -> ID {
        let mut buff = [0u8; ID_LEN];

        BigEndian::write_u32(&mut buff, ts as u32);

        buff[4] = self.machine_id[0];
        buff[5] = self.machine_id[1];
        buff[6] = self.machine_id[2];

        buff[7] = (self.pid >> 8) as u8;
        buff[8] = self.pid as u8;

        let i = self.counter.fetch_add(1, Ordering::SeqCst);
        buff[9] = (i >> 16) as u8;
        buff[10] = (i >> 8) as u8;
        buff[11] = (i) as u8;

        ID { val: buff }
    }
}

impl fmt::Debug for Generator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Generator {{counter: {:?}, machine_id: {:?}, pid: {:?}}}",
            self.counter, self.machine_id, self.pid
        )
    }
}

// ---

impl ID {
    pub fn encode(&self) -> String {
        self.encoding().unwrap().encode(&self.val)
    }

    pub fn machine(&self) -> [u8; 3] {
        [self.val[4], self.val[5], self.val[6]]
    }

    pub fn pid(&self) -> u16 {
        BigEndian::read_u16(&[self.val[7], self.val[8]])
    }

    pub fn time(&self) -> SystemTime {
        let ts = BigEndian::read_u32(&[self.val[0], self.val[1], self.val[2], self.val[3]]);

        UNIX_EPOCH + Duration::from_secs(ts as u64)
    }

    pub fn counter(&self) -> u32 {
        (self.val[9] as u32) << 16 | (self.val[10] as u32) << 8 | (self.val[11] as u32)
    }

    fn encoding(&self) -> Result<Encoding, SpecificationError> {
        let mut spec = Specification::new();
        spec.symbols.push_str("0123456789abcdefghijklmnopqrstuv");
        spec.encoding()
    }
}

impl fmt::Debug for ID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ID: {:?}", self.val)
    }
}

impl fmt::Display for ID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ID: {:?}", self.encode())
    }
}

impl PartialEq for ID {
    fn eq(&self, other: &ID) -> bool {
        self.val == other.val
    }
}

impl Eq for ID {}

impl PartialOrd for ID {
    fn partial_cmp(&self, other: &ID) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ID {
    fn cmp(&self, other: &ID) -> std::cmp::Ordering {
        self.val.cmp(&other.val)
    }
}

// ---

fn rand_int() -> AtomicUsize {
    let mut buff = [0u8; 3];

    thread_rng().fill_bytes(&mut buff);

    let x = (buff[0] as usize) << 16 | (buff[1] as usize) << 8 | buff[2] as usize;

    AtomicUsize::new(x)
}

fn get_pid() -> u32 {
    let mut pid = process::id();

    // If /proc/self/cpuset exists and is not /, we can assume that we are in a
    // form of container and use the content of cpuset xor-ed with the PID in
    // order get a reasonable machine global unique PID.
    match fs::read("/proc/self/cpuset") {
        Err(_) => pid,

        Ok(buff) => {
            let mut hasher = Hasher::new();
            hasher.update(buff.as_slice());
            let checksum = hasher.finalize();

            pid ^= checksum;

            pid
        }
    }
}

fn read_machine_id() -> [u8; 3] {
    let id = match platform_machine_id() {
        // XXX: https://github.com/rust-lang/rfcs/blob/master/text/0107-pattern-guards-with-bind-by-move.md
        Ok(x) => {
            if x.len() > 0 {
                x
            } else {
                hostname_string()
            }
        }

        _ => hostname_string(),
    };

    if id.len() <= 0 {
        let mut buff = [0u8; 3];
        thread_rng().fill_bytes(&mut buff);
        return buff;
    }

    let hash = md5::compute(id);
    return [hash[0], hash[1], hash[2]];
}

#[cfg(target_os = "linux")]
fn platform_machine_id() -> Result<String, io::Error> {
    // XXX: unlikely to work if read with an unpriviledged user
    let mut file = File::open("/sys/class/dmi/id/product_uuid")?;

    let mut contents = String::new();

    file.read_to_string(&mut contents)?;

    Ok(contents)
}

fn hostname_string() -> String {
    gethostname().into_string().unwrap()
}

// ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let total = 1e6 as u32;

        println!("Testing with {} ids", total);

        let mut g = new_generator();

        let mut previous_counter = 0;
        let mut previous_id = g.new_id().unwrap();

        for i in 0..total {
            let id = g.new_id().unwrap();

            assert!(
                previous_id < id,
                format!(
                    "{} ({:?}) not < {} ({:?})",
                    previous_id.encode(),
                    previous_id,
                    id.encode(),
                    id
                )
            );

            if i > 0 {
                assert_eq!(id.counter(), previous_counter + 1);
            }

            previous_counter = id.counter();

            {
                let x = id.encode();
                //println!("{:?}", x);
                assert_eq!(x.len(), 20);
            }

            assert_eq!(id.machine(), g.machine_id);

            previous_id = id;
        }
    }

    #[test]
    fn test_eq() {
        let mut g = new_generator();

        let a = g.new_id().unwrap();
        let b = g.new_id().unwrap();
        let c = g.new_id().unwrap();

        assert!(a == a);
        assert!(a <= a);
        assert!(a != b);
        assert!(a != c);

        assert!(a < b);
        assert!(b > a);
        assert!(b >= a);

        assert!(b < c);
        assert!(c > b);

        assert!(a < c);
        assert!(c > a);
    }
}
