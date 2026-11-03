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

fn identity() -> Identity {
    let mut noise = Noise::seeded();

    let software = if noise.coin() {
        "EasyEDA Pro"
    } else {
        "EasyEDA"
    };
    let version = format!(
        "v{}.{}.{}.{}",
        noise.between(2, 3),
        noise.between(1, 5),
        noise.between(1, 42),
        noise.between(0, 2)
    );
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    Identity {
        software,
        version,
        timestamp,
    }
}

/// Comment banner placed at the top of every Gerber file.
pub fn gerber_header() -> String {
    let info = identity();
    format!(
        "G04 {} {}, {}*\nG04 Gerber Generator version 0.3*\n",
        info.software, info.version, info.timestamp
    )
}

/// Comment banner placed at the top of a merged Excellon file.
pub fn drill_header(hole_type: &str, layer: &str) -> String {
    let info = identity();
    format!(
        ";TYPE={}\n;Layer: {}\n;{} {}, {}\n;Gerber Generator version 0.3\n",
        hole_type, layer, info.software, info.version, info.timestamp
    )
}

/// Ordering instructions shipped next to the converted layers.
pub fn order_guide() -> &'static str {
    "如何进行PCB下单\n\n请查看：\nhttps://prodocs.lceda.cn/cn/pcb/order-order-pcb/index.html"
}
