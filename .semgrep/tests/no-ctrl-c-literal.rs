fn literals() {
    // ruleid: no-ctrl-c-literal
    let _ = b"\x03";
    // ruleid: no-ctrl-c-literal
    let _ = "\u{3}";
    // ruleid: no-ctrl-c-literal
    let _ = '\x03';
    // ruleid: no-ctrl-c-literal
    let _ = b"ab\x03cd";
    // ok: no-ctrl-c-literal
    let _ = b"\x1b[A";
    // ok: no-ctrl-c-literal
    let _ = vec![0x03];
    // ok: no-ctrl-c-literal
    let _ = "\x1b03";
}
