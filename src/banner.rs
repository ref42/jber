/* src/banner.rs */

//! Generated comment headers for the converted files, plus the order guide.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;

/// Small xorshift generator, so the header varies per run without pulling in a
/// random-number dependency.
struct Noise(u64);

impl Noise {
    fn seeded() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0);
        let seed = nanos ^ (u64::from(std::process::id()) << 32) | 1;
        Self(seed)
    }

    fn from_seed(seed: u64) -> Self {
        // Mix before forcing oddness so neighbouring seeds do not collapse
        // onto the same state.
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low + 1)
    }

    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

struct Identity {
    software: &'static str,
    version: String,
    timestamp: String,
}

/// Produces the randomized parts of the headers. With a fixed seed and epoch
/// the whole conversion becomes reproducible, which the regression tests use.
pub struct Stamp {
    noise: Noise,
    timestamp: String,
}

impl Stamp {
    pub fn new(seed: Option<u64>, epoch: Option<i64>) -> Self {
        let noise = match seed {
            Some(seed) => Noise::from_seed(seed),
            None => Noise::seeded(),
        };

        let timestamp = match epoch {
            Some(seconds) => chrono::DateTime::from_timestamp(seconds, 0)
                .map(|moment| moment.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "1970-01-01 00:00:00".to_string()),
            None => Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        };

        Self { noise, timestamp }
    }

    fn identity(&mut self) -> Identity {
        let software = if self.noise.coin() {
            "EasyEDA Pro"
        } else {
            "EasyEDA"
        };
        let version = format!(
            "v{}.{}.{}.{}",
            self.noise.between(2, 3),
            self.noise.between(1, 5),
            self.noise.between(1, 42),
            self.noise.between(0, 2)
        );

        Identity {
            software,
            version,
            timestamp: self.timestamp.clone(),
        }
    }

    /// Comment banner placed at the top of every Gerber file.
    pub fn gerber_header(&mut self) -> String {
        let info = self.identity();
        format!(
            "G04 {} {}, {}*\nG04 Gerber Generator version 0.3*\n",
            info.software, info.version, info.timestamp
        )
    }

    /// Comment banner placed at the top of a merged Excellon file.
    pub fn drill_header(&mut self, hole_type: &str, layer: &str) -> String {
        let info = self.identity();
        format!(
            ";TYPE={}\n;Layer: {}\n;{} {}, {}\n;Gerber Generator version 0.3\n",
            hole_type, layer, info.software, info.version, info.timestamp
        )
    }
}

/// Ordering instructions shipped next to the converted layers.
pub fn order_guide() -> &'static str {
    "如何进行PCB下单\n\n请查看：\nhttps://prodocs.lceda.cn/cn/pcb/order-order-pcb/index.html"
}
