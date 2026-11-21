//! Minimaler zlib/DEFLATE-Decoder (RFC 1950/1951) zum Entpacken der
//! eingebetteten Fixture-Bibliothek. Nach dem Vorbild von zlibs "puff.c".
//! Bewusst ohne Abhängigkeiten und nur einmal beim ersten Öffnen benutzt.

const MAXBITS: usize = 15;

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

struct Huffman {
    count: [u16; MAXBITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Self {
        let mut count = [0u16; MAXBITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut offs = [0u16; MAXBITS + 1];
        for len in 1..MAXBITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Self { count, symbol }
    }
}

struct State<'a> {
    input: &'a [u8],
    pos: usize,
    bitbuf: u32,
    bitcnt: u32,
    out: Vec<u8>,
}

impl State<'_> {
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        let mut val = self.bitbuf;
        while self.bitcnt < need {
            let b = *self.input.get(self.pos).ok_or("Daten zu kurz")? as u32;
            self.pos += 1;
            val |= b << self.bitcnt;
            self.bitcnt += 8;
        }
        self.bitbuf = val >> need;
        self.bitcnt -= need;
        Ok(val & ((1u32 << need) - 1))
    }

    fn decode(&mut self, h: &Huffman) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAXBITS {
            code |= self.bits(1)? as i32;
            let count = h.count[len] as i32;
            if code - count < first {
                return h
                    .symbol
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or_else(|| "ungültiger Huffman-Code".to_string());
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("ungültiger Huffman-Code".into())
    }

    fn stored(&mut self) -> Result<(), String> {
        self.bitbuf = 0;
        self.bitcnt = 0;
        let hdr = self.input.get(self.pos..self.pos + 4).ok_or("Daten zu kurz")?;
        let len = u16::from_le_bytes([hdr[0], hdr[1]]);
        let nlen = u16::from_le_bytes([hdr[2], hdr[3]]);
        if len != !nlen {
            return Err("Stored-Block beschädigt".into());
        }
        self.pos += 4;
        let data = self
            .input
            .get(self.pos..self.pos + len as usize)
            .ok_or("Daten zu kurz")?;
        self.out.extend_from_slice(data);
        self.pos += len as usize;
        Ok(())
    }

    fn codes(&mut self, lencode: &Huffman, distcode: &Huffman) -> Result<(), String> {
        loop {
            let sym = self.decode(lencode)? as usize;
            if sym < 256 {
                self.out.push(sym as u8);
            } else if sym == 256 {
                return Ok(());
            } else {
                let s = sym - 257;
                if s >= 29 {
                    return Err("ungültige Länge".into());
                }
                let len = LBASE[s] as usize + self.bits(LEXT[s] as u32)? as usize;
                let ds = self.decode(distcode)? as usize;
                if ds >= 30 {
                    return Err("ungültige Distanz".into());
                }
                let dist = DBASE[ds] as usize + self.bits(DEXT[ds] as u32)? as usize;
                if dist > self.out.len() {
                    return Err("Distanz zu weit".into());
                }
                let start = self.out.len() - dist;
                for i in 0..len {
                    let b = self.out[start + i];
                    self.out.push(b);
                }
            }
        }
    }

    fn fixed(&mut self) -> Result<(), String> {
        let mut lengths = [0u8; 288];
        for (i, l) in lengths.iter_mut().enumerate() {
            *l = match i {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        let lencode = Huffman::new(&lengths);
        let distcode = Huffman::new(&[5u8; 30]);
        self.codes(&lencode, &distcode)
    }

    fn dynamic(&mut self) -> Result<(), String> {
        let nlen = self.bits(5)? as usize + 257;
        let ndist = self.bits(5)? as usize + 1;
        let ncode = self.bits(4)? as usize + 4;
        if nlen > 286 || ndist > 30 {
            return Err("ungültige Code-Anzahl".into());
        }
        let mut lengths = [0u8; 320];
        for &o in ORDER.iter().take(ncode) {
            lengths[o] = self.bits(3)? as u8;
        }
        let lencode = Huffman::new(&lengths[..19]);

        let mut index = 0;
        while index < nlen + ndist {
            let sym = self.decode(&lencode)?;
            if sym < 16 {
                lengths[index] = sym as u8;
                index += 1;
            } else {
                let (val, rep) = match sym {
                    16 => {
                        if index == 0 {
                            return Err("Wiederholung ohne Vorgänger".into());
                        }
                        (lengths[index - 1], 3 + self.bits(2)? as usize)
                    }
                    17 => (0, 3 + self.bits(3)? as usize),
                    _ => (0, 11 + self.bits(7)? as usize),
                };
                if index + rep > nlen + ndist {
                    return Err("zu viele Längen".into());
                }
                for _ in 0..rep {
                    lengths[index] = val;
                    index += 1;
                }
            }
        }
        let lencode = Huffman::new(&lengths[..nlen]);
        let distcode = Huffman::new(&lengths[nlen..nlen + ndist]);
        self.codes(&lencode, &distcode)
    }
}

pub fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 2 || data[0] & 0x0F != 8 {
        return Err("kein zlib-Stream".into());
    }
    let mut s = State {
        input: &data[2..],
        pos: 0,
        bitbuf: 0,
        bitcnt: 0,
        out: Vec::with_capacity(data.len() * 8),
    };
    loop {
        let last = s.bits(1)?;
        match s.bits(2)? {
            0 => s.stored()?,
            1 => s.fixed()?,
            2 => s.dynamic()?,
            _ => return Err("ungültiger Blocktyp".into()),
        }
        if last == 1 {
            return Ok(s.out);
        }
    }
}
