fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(mediaservo_gates::run(&args));
}
