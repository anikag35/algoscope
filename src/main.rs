//! Entry point: argument parsing, terminal setup, and the event loop.
//!
//! The loop is deliberately plain. It redraws, waits for input with a timeout
//! equal to whatever is left of the frame budget, then advances playback by the
//! wall time that actually elapsed. Input is therefore handled the moment it
//! arrives rather than at the next frame boundary, while playback speed stays
//! independent of the render rate.

use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::crossterm::event::{self, Event};

use algoscope::algo::sort::Algorithm;
use algoscope::app::{App, DEFAULT_SIZE, MAX_SIZE, MIN_SIZE};
use algoscope::ui;
use algoscope::ui::theme::Theme;

/// Target frame interval, about 60 fps.
///
/// This caps redraws, not playback: at the top speed a single frame applies
/// several hundred steps, which is what makes a large array resolve smoothly
/// instead of one comparison per frame.
const FRAME: Duration = Duration::from_millis(16);

const HELP: &str = "\
algoscope - visualize sorting algorithms in the terminal

USAGE:
    algoscope [OPTIONS]

OPTIONS:
    -a, --algorithm <NAME>  bubble, insertion or selection [default: bubble]
    -n, --size <N>          number of elements [default: 48]
    -s, --seed <N>          seed the shuffle, for a reproducible run
        --ascii             ASCII glyphs and no colour
    -h, --help              print this message
    -V, --version           print the version

KEYS:
    space  play/pause      left/right  step        up/down  speed
    tab    next algorithm  r           reshuffle   ?        all keys
    q      quit

Switching algorithm keeps the same array, so the comparison counts of two
algorithms can be compared directly.
";

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        // --help and --version are a successful exit, not an error.
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("algoscope: {message}\n\nTry 'algoscope --help'.");
            return ExitCode::FAILURE;
        }
    };

    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("algoscope: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> io::Result<()> {
    // `init` enables raw mode, switches to the alternate screen, and installs
    // a panic hook that undoes both. Without that hook a panic mid-render
    // would leave the user's terminal in raw mode with no echo.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, options);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, options: Options) -> io::Result<()> {
    let Options {
        algorithm,
        size,
        seed,
        theme,
        ..
    } = options;
    let mut app = App::new(algorithm, size, seed);
    let mut last_tick = Instant::now();

    while !app.should_quit() {
        terminal.draw(|frame| ui::draw(frame, &app, &theme))?;

        // Wait out the rest of the frame, but no longer: an event arriving
        // early wakes the loop immediately.
        let timeout = FRAME.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                // A resize only changes the layout, which the next draw picks
                // up on its own. The array is never resized to match.
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        // Advance by however long it actually took. Fractional steps are
        // carried in the accumulator, so a tiny `dt` from an early wake-up
        // loses nothing.
        let now = Instant::now();
        app.tick(now.duration_since(last_tick));
        last_tick = now;
    }

    Ok(())
}

#[derive(Debug)]
struct Options {
    algorithm: Algorithm,
    size: usize,
    seed: u64,
    theme: Theme,
}

impl Options {
    /// Parse arguments, or return `Ok(None)` if the program should just exit.
    ///
    /// Hand-rolled to keep the dependency tree at three crates; the surface is
    /// four flags and does not justify an argument parser.
    fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<Option<Self>, String> {
        let mut algorithm = Algorithm::default();
        let mut size = DEFAULT_SIZE;
        let mut ascii = false;
        // Varies per launch unless pinned, so repeated runs are not identical.
        let mut seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    print!("{HELP}");
                    return Ok(None);
                }
                "-V" | "--version" => {
                    println!("algoscope {}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                "--ascii" => ascii = true,
                "-a" | "--algorithm" => {
                    let value = next_value(&mut args, &arg)?;
                    algorithm = parse_algorithm(&value)?;
                }
                "-n" | "--size" => {
                    let value = next_value(&mut args, &arg)?;
                    let parsed: usize = value
                        .parse()
                        .map_err(|_| format!("--size wants a number, got '{value}'"))?;
                    if !(MIN_SIZE..=MAX_SIZE).contains(&parsed) {
                        return Err(format!(
                            "--size must be between {MIN_SIZE} and {MAX_SIZE}, got {parsed}"
                        ));
                    }
                    size = parsed;
                }
                "-s" | "--seed" => {
                    let value = next_value(&mut args, &arg)?;
                    seed = value
                        .parse()
                        .map_err(|_| format!("--seed wants a number, got '{value}'"))?;
                }
                other => return Err(format!("unknown argument '{other}'")),
            }
        }

        let theme = if ascii { Theme::minimal() } else { Theme::detect() };

        Ok(Some(Self {
            algorithm,
            size,
            seed,
            theme,
        }))
    }
}

fn next_value<I: Iterator<Item = String>>(args: &mut I, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_algorithm(name: &str) -> Result<Algorithm, String> {
    // Prefix matching, so `-a ins` works.
    let lowered = name.to_ascii_lowercase();
    Algorithm::ALL
        .into_iter()
        .find(|a| a.name().to_ascii_lowercase().starts_with(&lowered) && !lowered.is_empty())
        .ok_or_else(|| {
            format!("unknown algorithm '{name}' (try bubble, insertion or selection)")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_are_sane() {
        let options = parse(&[]).unwrap().unwrap();
        assert_eq!(options.algorithm, Algorithm::Bubble);
        assert_eq!(options.size, DEFAULT_SIZE);
    }

    #[test]
    fn flags_are_read() {
        let options = parse(&["--algorithm", "selection", "--size", "100", "--seed", "42"])
            .unwrap()
            .unwrap();
        assert_eq!(options.algorithm, Algorithm::Selection);
        assert_eq!(options.size, 100);
        assert_eq!(options.seed, 42);
    }

    #[test]
    fn short_flags_match_long_ones() {
        let long = parse(&["--algorithm", "insertion", "--size", "20"])
            .unwrap()
            .unwrap();
        let short = parse(&["-a", "insertion", "-n", "20"]).unwrap().unwrap();
        assert_eq!(long.algorithm, short.algorithm);
        assert_eq!(long.size, short.size);
    }

    #[test]
    fn algorithm_names_can_be_abbreviated() {
        for (input, want) in [
            ("bubble", Algorithm::Bubble),
            ("BUBBLE", Algorithm::Bubble),
            ("ins", Algorithm::Insertion),
            ("s", Algorithm::Selection),
        ] {
            assert_eq!(parse_algorithm(input).unwrap(), want, "for {input}");
        }
        assert!(parse_algorithm("quick").is_err());
        assert!(parse_algorithm("").is_err());
    }

    #[test]
    fn seed_defaults_to_something_that_varies() {
        // Pinning the seed is opt-in; the default should not be a constant, or
        // every launch would show the identical shuffle.
        let options = parse(&[]).unwrap().unwrap();
        assert_ne!(options.seed, 0);
    }

    #[test]
    fn bad_input_is_rejected_with_a_message() {
        for args in [
            vec!["--size", "0"],
            vec!["--size", "99999"],
            vec!["--size", "many"],
            vec!["--seed", "soon"],
            vec!["--algorithm"],
            vec!["--size"],
            vec!["--nonsense"],
        ] {
            let error = parse(&args).expect_err(&format!("{args:?} should be rejected"));
            assert!(!error.is_empty(), "error message should not be empty");
        }
    }

    #[test]
    fn help_and_version_exit_without_options() {
        assert!(parse(&["--help"]).unwrap().is_none());
        assert!(parse(&["-V"]).unwrap().is_none());
    }

    #[test]
    fn ascii_flag_forces_the_minimal_theme() {
        let options = parse(&["--ascii"]).unwrap().unwrap();
        assert!(!options.theme.unicode);
    }
}
