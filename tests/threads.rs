//! The output is the same bytes however many threads the work is cut into.

mod common;

use common::{fixture, gradient_image};

#[test]
fn one_thread_and_many_give_the_same_bytes() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = std::env::temp_dir().join(format!("rehue-threads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // odd sizes, so runs end mid-row and mid-block
    let wall = dir.join("wall.png");
    gradient_image((333, 211)).save(&wall).unwrap();
    let scheme = fixture("solarized-dark.yaml");
    let wall = wall.to_str().unwrap();
    let scheme = scheme.to_str().unwrap();
    let cases: [&[&str]; 7] = [
        &["-H", "1"],
        &["-l", "snap"],
        &["-H", "1", "--dither", "checker,scale=3"],
        &["--dither", "checker,sample=pixel,scale=2"],
        &["-l", "newsprint"],
        &["--dither", "diffuse,scale=2"],
        &["-H", "1", "--lut", "hald", "--lut-level", "4"],
    ];
    for (k, args) in cases.iter().enumerate() {
        let mut outputs = Vec::new();
        for threads in ["1", "5"] {
            let out = dir.join(format!("{k}-{threads}"));
            let run = std::process::Command::new(bin)
                .args(["wal", "-i", wall, "-s", scheme])
                .args(*args)
                .arg("-o")
                .arg(&out)
                .env("REHUE_THREADS", threads)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&out)
                .unwrap()
                .map(|e| e.unwrap().path())
                .map(|p| {
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    (name, std::fs::read(&p).unwrap())
                })
                .collect();
            files.sort();
            outputs.push(files);
        }
        assert!(outputs[0].iter().any(|(n, _)| n == "wallpaper.png"));
        for ((name, one), (_, many)) in outputs[0].iter().zip(&outputs[1]) {
            assert!(
                one == many,
                "{args:?}: {name} differs between 1 and 5 threads"
            );
        }
        assert_eq!(outputs[0].len(), outputs[1].len());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
