pub fn human_bytes(b: impl Into<u128>) -> String {
    let mut n: f64 = b.into() as f64;
    let units = ["B","KB","MB","GB","TB","PB"]; let mut u=0;
    while n >= 1024.0 && u < units.len()-1 { n/=1024.0; u+=1; }
    format!("{:.2} {}", n, units[u])
}
