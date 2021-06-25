#[no_mangle]
pub extern "C" fn hello(x: u64) -> u64 {
    colatz(x, 1)
}

fn colatz(x: u64, n: u64) -> u64 {
    if x == 1 || x == 0 {
        return n;
    } else if x % 2 == 0 {
        return colatz(x / 2, n + 1);
    } else {
        return colatz(x * 3 + 1, n + 1);
    }
}
