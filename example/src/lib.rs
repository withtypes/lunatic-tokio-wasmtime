#[no_mangle]
pub extern "C" fn hello(x: u64) -> u64 {
    let mut res = 1;
    for i in 1..(x * x) {
        res *= i;
    }
    res
}
