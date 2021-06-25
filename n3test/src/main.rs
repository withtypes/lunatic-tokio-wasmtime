fn main() {
    let mut x : u64 = 1;
    let t = std::time::Instant::now();
    for i in 1..10000 {
        for j in 1..(i * i) {
            x *= j;
        }
    }

    println!("{}, {}", x, t.elapsed().as_millis());
}
