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

/// Produces the randomized parts of the headers.
pub struct Stamp {
    noise: Noise,
    timestamp: String,
}

impl Stamp {
    pub fn new() -> Self {
        Self {
            noise: Noise::seeded(),
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }
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
    "PCB 下单说明\n\n上传本压缩包即可下单。\n参考：\nhttps://prodocs.lceda.cn/cn/pcb/order-order-pcb/index.html"
}
