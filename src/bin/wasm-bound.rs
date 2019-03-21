use failure::ResultExt;
use std::io::{self, Write};
use std::path;
use std::process;

fn main() {
    if let Err(e) = try_main() {
        eprintln!("error: {}", e);
        for c in e.iter_chain().skip(1) {
            eprintln!("  caused by {}", c);
        }
        eprintln!("{}", e.backtrace());
        process::exit(1)
    }
}

fn try_main() -> Result<(), failure::Error> {
    let matches = parse_args();

    let mut opts = wasm_bound::Options::default();

    opts.input = path::PathBuf::from(matches.value_of("input").unwrap());

    opts.functions = matches
        .values_of("function")
        .map(|fs| fs.map(|f| f.to_string()).collect())
        .unwrap_or(vec![]);

    let module = wasm_bound::snip(opts).context("failed to snip functions from wasm module")?;

    if let Some(output) = matches.value_of("output") {
        module
            .emit_wasm_file(output)
            .with_context(|_| format!("failed to emit bounded wasm to {}", output))?;
    } else {
        let wasm = module
            .emit_wasm()
            .context("failed to re-compile bounded module to wasm")?;

        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        stdout
            .write_all(&wasm)
            .context("failed to write wasm to stdout")?;
    }

    Ok(())
}

fn parse_args() -> clap::ArgMatches<'static> {
    clap::App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .long_about(
            "
`wasm-bound` injects aproxement instruction limites in to wasam
files. If the limit is exceeded `unreachable` is triggered.

Sometimes you want to run a wasm module but want to make sure it can't
speed too much time executing wasm instructions before returning. The
wasm-bound file will inject aproximet instruction counting your wasm
module so that you wont get stuck in a loop evere agine!
",
        )
        .arg(
            clap::Arg::with_name("output")
                .short("o")
                .long("output")
                .takes_value(true)
                .help("The path to write the output wasm file to. Defaults to stdout."),
        )
        .arg(
            clap::Arg::with_name("input")
                .required(true)
                .help("The input wasm file containing the function(s) to bound."),
        )
        .arg(clap::Arg::with_name("function").multiple(true).help(
            "The specific function(s) to skip. These must match \
             exactly.",
        ))
        .get_matches()
}
