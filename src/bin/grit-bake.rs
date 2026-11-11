use std::process::ExitCode;

use grit::bake::{self, Asset, Map, Options};

const USAGE: &str = "usage: grit-bake MODEL.glb MAP.json OUT.grvx --size SIDE [--scale FACTOR]";

struct Asked {
    model: String,
    map: String,
    out: String,
    options: Options,
}

fn number(value: Option<String>, flag: &str) -> Result<f64, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a number"))?;
    value
        .parse::<f64>()
        .map_err(|_| format!("{flag} {value:?} is not a number"))
}

fn asked(mut args: impl Iterator<Item = String>) -> Result<Asked, String> {
    let mut files = Vec::new();
    let mut size = None;
    let mut scale = 1.0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => size = Some(number(args.next(), "--size")?),
            "--scale" => scale = number(args.next(), "--scale")?,
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            _ => files.push(arg),
        }
    }
    let [model, map, out] = <[String; 3]>::try_from(files)
        .map_err(|files| format!("three files are needed, not {}", files.len()))?;
    let size = size.ok_or_else(|| "--size is missing".to_owned())?;
    Ok(Asked {
        model,
        map,
        out,
        options: Options { size, scale },
    })
}

fn run(asked: &Asked) -> Result<Asset, String> {
    let read = |path: &str| std::fs::read(path).map_err(|why| format!("{path}: {why}"));
    let model =
        bake::read_glb(&read(&asked.model)?).map_err(|why| format!("{}: {why}", asked.model))?;
    let map = Map::read(&read(&asked.map)?).map_err(|why| format!("{}: {why}", asked.map))?;
    let asset = bake::bake(&model, &map, asked.options).map_err(|why| why.to_string())?;
    std::fs::write(&asked.out, asset.write()).map_err(|why| format!("{}: {why}", asked.out))?;
    Ok(asset)
}

fn main() -> ExitCode {
    let asked = match asked(std::env::args().skip(1)) {
        Ok(asked) => asked,
        Err(why) => {
            eprintln!("{why}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&asked) {
        Ok(asset) => {
            let mut words: Vec<(u32, usize)> = Vec::new();
            for word in asset.cells.iter().flatten() {
                match words.iter_mut().find(|(known, _)| known == word) {
                    Some((_, count)) => *count += 1,
                    None => words.push((*word, 1)),
                }
            }
            let [wide, deep, tall] = asset.size;
            println!(
                "{}: box {wide} x {deep} x {tall}, anchor {:?}, {} cells filled",
                asked.out,
                asset.anchor(),
                asset.filled()
            );
            for (word, count) in words {
                println!("  word {word:#010x}: {count} cells");
            }
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("grit-bake: {why}");
            ExitCode::FAILURE
        }
    }
}
