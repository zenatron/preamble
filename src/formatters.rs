pub fn format_track_duration(duration_millis: Option<u32>) -> Option<String> {
    duration_millis.map(|mut d| {
        d /= 1000;
        let mins = d / 60;
        let secs = d % 60;
        format!("{}:{:02}", mins, secs)
    })
}

pub fn format_thou(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}