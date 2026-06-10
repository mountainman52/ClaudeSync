use std::collections::{BinaryHeap, HashMap};
use std::io::{Read, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::error::{CsError, Result};

/// Compresses text with the named algorithm, returning a transport-safe string.
/// Unknown algorithms pass content through unchanged (matches the Python port).
pub fn compress_content(content: &str, algorithm: &str) -> Result<String> {
    match algorithm {
        "zlib" => zlib_compress(content),
        "bz2" => bz2_compress(content),
        "lzma" => lzma_compress(content),
        "brotli" => brotli_compress(content),
        "dictionary" => dictionary_compress(content),
        "rle" => rle_compress(content),
        "huffman" => huffman_compress(content),
        "lzw" => lzw_compress(content),
        "pack" => Ok(content.to_string()),
        _ => Ok(content.to_string()),
    }
}

pub fn decompress_content(compressed: &str, algorithm: &str) -> Result<String> {
    match algorithm {
        "zlib" => zlib_decompress(compressed),
        "bz2" => bz2_decompress(compressed),
        "lzma" => lzma_decompress(compressed),
        "brotli" => brotli_decompress(compressed),
        "dictionary" => dictionary_decompress(compressed),
        "rle" => rle_decompress(compressed),
        "huffman" => huffman_decompress(compressed),
        "lzw" => lzw_decompress(compressed),
        "pack" => Ok(compressed.to_string()),
        _ => Ok(compressed.to_string()),
    }
}

fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

fn unb64(text: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(text.trim())
        .map_err(|e| CsError::Other(format!("Invalid base64: {e}")))
}

fn utf8(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|e| CsError::Other(format!("Invalid UTF-8: {e}")))
}

// Zlib
fn zlib_compress(text: &str) -> Result<String> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(text.as_bytes())?;
    Ok(b64(&enc.finish()?))
}

fn zlib_decompress(compressed: &str) -> Result<String> {
    let mut dec = flate2::read::ZlibDecoder::new(std::io::Cursor::new(unb64(compressed)?));
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    utf8(out)
}

// BZ2
fn bz2_compress(text: &str) -> Result<String> {
    let mut enc =
        bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    enc.write_all(text.as_bytes())?;
    Ok(b64(&enc.finish()?))
}

fn bz2_decompress(compressed: &str) -> Result<String> {
    let mut dec = bzip2::read::BzDecoder::new(std::io::Cursor::new(unb64(compressed)?));
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    utf8(out)
}

// LZMA (xz container, matching Python's lzma.compress default)
fn lzma_compress(text: &str) -> Result<String> {
    let mut input = std::io::Cursor::new(text.as_bytes());
    let mut out = Vec::new();
    lzma_rs::xz_compress(&mut input, &mut out)
        .map_err(|e| CsError::Other(format!("LZMA compression failed: {e}")))?;
    Ok(b64(&out))
}

fn lzma_decompress(compressed: &str) -> Result<String> {
    let bytes = unb64(compressed)?;
    let mut input = std::io::Cursor::new(bytes);
    let mut out = Vec::new();
    lzma_rs::xz_decompress(&mut input, &mut out)
        .map_err(|e| CsError::Other(format!("LZMA decompression failed: {e}")))?;
    utf8(out)
}

// Brotli
fn brotli_compress(text: &str) -> Result<String> {
    let mut input = std::io::Cursor::new(text.as_bytes());
    let mut out = Vec::new();
    let params = brotli::enc::BrotliEncoderParams::default();
    brotli::BrotliCompress(&mut input, &mut out, &params)?;
    Ok(b64(&out))
}

fn brotli_decompress(compressed: &str) -> Result<String> {
    let bytes = unb64(compressed)?;
    let mut input = std::io::Cursor::new(bytes);
    let mut out = Vec::new();
    brotli::BrotliDecompress(&mut input, &mut out)?;
    utf8(out)
}

// Dictionary-based compression
fn dictionary_compress(text: &str) -> Result<String> {
    let mut dictionary: HashMap<String, String> = HashMap::new();
    let mut compressed = Vec::new();
    for word in text.split_whitespace() {
        let next_id = dictionary.len().to_string();
        let id = dictionary
            .entry(word.to_string())
            .or_insert(next_id)
            .clone();
        compressed.push(id);
    }
    Ok(serde_json::to_string(&json!({
        "dict": dictionary,
        "compressed": compressed.join(" "),
    }))?)
}

fn dictionary_decompress(compressed: &str) -> Result<String> {
    let data: Value = serde_json::from_str(compressed)?;
    let dict = data["dict"]
        .as_object()
        .ok_or_else(|| CsError::Other("Bad dictionary payload".into()))?;
    let reverse: HashMap<&str, &str> = dict
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|id| (id, k.as_str())))
        .collect();
    let tokens = data["compressed"]
        .as_str()
        .ok_or_else(|| CsError::Other("Bad dictionary payload".into()))?;
    let words: Result<Vec<&str>> = tokens
        .split_whitespace()
        .map(|t| {
            reverse
                .get(t)
                .copied()
                .ok_or_else(|| CsError::Other(format!("Unknown token: {t}")))
        })
        .collect();
    Ok(words?.join(" "))
}

// Run-length encoding
fn rle_compress(text: &str) -> Result<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut compressed: Vec<(String, usize)> = Vec::new();
    let mut count = 1usize;
    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            count += 1;
        } else {
            compressed.push((chars[i - 1].to_string(), count));
            count = 1;
        }
    }
    if let Some(last) = chars.last() {
        compressed.push((last.to_string(), count));
    }
    Ok(serde_json::to_string(&compressed)?)
}

fn rle_decompress(compressed: &str) -> Result<String> {
    let pairs: Vec<(String, usize)> = serde_json::from_str(compressed)?;
    Ok(pairs
        .into_iter()
        .map(|(c, n)| c.repeat(n))
        .collect::<String>())
}

// Huffman coding
#[derive(Eq, PartialEq)]
struct HuffmanNode {
    freq: usize,
    char: Option<char>,
    left: Option<Box<HuffmanNode>>,
    right: Option<Box<HuffmanNode>>,
}

impl Ord for HuffmanNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap behavior in BinaryHeap
        other.freq.cmp(&self.freq)
    }
}

impl PartialOrd for HuffmanNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn generate_codes(node: &HuffmanNode, code: String, codes: &mut HashMap<char, String>) {
    if let Some(c) = node.char {
        codes.insert(c, code);
        return;
    }
    if let Some(left) = &node.left {
        generate_codes(left, format!("{code}0"), codes);
    }
    if let Some(right) = &node.right {
        generate_codes(right, format!("{code}1"), codes);
    }
}

fn huffman_compress(text: &str) -> Result<String> {
    let mut freq: HashMap<char, usize> = HashMap::new();
    for c in text.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }
    if freq.is_empty() {
        return Err(CsError::Other("Cannot huffman-compress empty text".into()));
    }

    let mut heap: BinaryHeap<HuffmanNode> = freq
        .iter()
        .map(|(&c, &f)| HuffmanNode {
            freq: f,
            char: Some(c),
            left: None,
            right: None,
        })
        .collect();

    while heap.len() > 1 {
        let left = heap.pop().unwrap();
        let right = heap.pop().unwrap();
        heap.push(HuffmanNode {
            freq: left.freq + right.freq,
            char: None,
            left: Some(Box::new(left)),
            right: Some(Box::new(right)),
        });
    }

    let root = heap.pop().unwrap();
    let mut codes: HashMap<char, String> = HashMap::new();
    generate_codes(&root, String::new(), &mut codes);
    // Single distinct character: assign code "0" so encoding is non-empty
    if codes.values().any(|c| c.is_empty()) {
        for code in codes.values_mut() {
            *code = "0".to_string();
        }
    }

    let mut encoded = String::new();
    for c in text.chars() {
        encoded.push_str(&codes[&c]);
    }
    let padding = 8 - encoded.len() % 8;
    encoded.push_str(&"0".repeat(padding));

    let mut bytes = Vec::with_capacity(encoded.len() / 8);
    for chunk in encoded.as_bytes().chunks(8) {
        let s = std::str::from_utf8(chunk).unwrap();
        bytes.push(u8::from_str_radix(s, 2).unwrap());
    }

    let tree: HashMap<String, String> = codes
        .into_iter()
        .map(|(c, code)| (c.to_string(), code))
        .collect();
    Ok(serde_json::to_string(&json!({
        "tree": tree,
        "padding": padding,
        "data": b64(&bytes),
    }))?)
}

fn huffman_decompress(compressed: &str) -> Result<String> {
    let data: Value = serde_json::from_str(compressed)?;
    let tree_obj = data["tree"]
        .as_object()
        .ok_or_else(|| CsError::Other("Bad huffman payload".into()))?;
    let tree: HashMap<String, String> = tree_obj
        .iter()
        .filter_map(|(c, code)| code.as_str().map(|code| (code.to_string(), c.clone())))
        .collect();
    let padding = data["padding"].as_u64().unwrap_or(0) as usize;
    let bytes = unb64(
        data["data"]
            .as_str()
            .ok_or_else(|| CsError::Other("Bad huffman payload".into()))?,
    )?;

    let mut binary = String::with_capacity(bytes.len() * 8);
    for byte in &bytes {
        binary.push_str(&format!("{byte:08b}"));
    }
    if padding > 0 && binary.len() >= padding {
        binary.truncate(binary.len() - padding);
    }

    let mut decoded = String::new();
    let mut code = String::new();
    for bit in binary.chars() {
        code.push(bit);
        if let Some(c) = tree.get(&code) {
            decoded.push_str(c);
            code.clear();
        }
    }
    Ok(decoded)
}

// LZW compression. Like the Python original, codes are emitted as single
// bytes, so inputs producing more than 256 dictionary entries fail.
fn lzw_compress(text: &str) -> Result<String> {
    let mut dictionary: HashMap<String, usize> =
        (0..256).map(|i| ((i as u8 as char).to_string(), i)).collect();
    let mut result: Vec<usize> = Vec::new();
    let mut w = String::new();
    for c in text.chars() {
        let wc = format!("{w}{c}");
        if dictionary.contains_key(&wc) {
            w = wc;
        } else {
            result.push(dictionary[&w]);
            dictionary.insert(wc, dictionary.len());
            w = c.to_string();
        }
    }
    if !w.is_empty() {
        result.push(dictionary[&w]);
    }
    let bytes: Result<Vec<u8>> = result
        .into_iter()
        .map(|code| {
            u8::try_from(code)
                .map_err(|_| CsError::Other(format!("LZW code out of byte range: {code}")))
        })
        .collect();
    Ok(b64(&bytes?))
}

fn lzw_decompress(compressed: &str) -> Result<String> {
    let bytes = unb64(compressed)?;
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let mut dictionary: HashMap<usize, String> =
        (0..256).map(|i| (i, (i as u8 as char).to_string())).collect();
    let mut result = Vec::new();
    let mut w = dictionary[&(bytes[0] as usize)].clone();
    result.push(w.clone());
    for &byte in &bytes[1..] {
        let k = byte as usize;
        let entry = if let Some(e) = dictionary.get(&k) {
            e.clone()
        } else if k == dictionary.len() {
            format!("{w}{}", w.chars().next().unwrap())
        } else {
            return Err(CsError::Other(format!("Bad compressed k: {k}")));
        };
        result.push(entry.clone());
        let next = format!("{w}{}", entry.chars().next().unwrap());
        dictionary.insert(dictionary.len(), next);
        w = entry;
    }
    Ok(result.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "hello hello hello world, this is a compression test! \
                          aaaaabbbbbccccc 1234567890";

    #[test]
    fn roundtrip_all_algorithms() {
        for algo in [
            "zlib", "bz2", "lzma", "brotli", "rle", "huffman", "pack", "none",
        ] {
            let compressed = compress_content(SAMPLE, algo).unwrap();
            let decompressed = decompress_content(&compressed, algo).unwrap();
            assert_eq!(decompressed, SAMPLE, "roundtrip failed for {algo}");
        }
    }

    #[test]
    fn roundtrip_lzw_small_input() {
        // LZW emits single-byte codes (like the Python original), so it only
        // works for inputs that never reuse a multi-character sequence.
        let text = "abcdefg";
        let compressed = compress_content(text, "lzw").unwrap();
        let decompressed = decompress_content(&compressed, "lzw").unwrap();
        assert_eq!(decompressed, text);
    }

    #[test]
    fn lzw_rejects_large_inputs() {
        assert!(compress_content(SAMPLE, "lzw").is_err());
    }

    #[test]
    fn roundtrip_dictionary() {
        // Dictionary compression normalizes whitespace (like the Python version)
        let text = "the quick brown fox the quick brown fox";
        let compressed = compress_content(text, "dictionary").unwrap();
        let decompressed = decompress_content(&compressed, "dictionary").unwrap();
        assert_eq!(decompressed, text);
    }
}
