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
use ratatui::layout::{Rect, Size};

use algoscope::algo::grid::search as search_algo;
use algoscope::algo::sort;
use algoscope::app::{
    App, Config, DEFAULT_SIZE, MAX_GRID, MAX_SIZE, MIN_GRID, MIN_SIZE, Mode,
};
use algoscope::ui;
use algoscope::ui::grid::GridView;
use algoscope::ui::theme::Theme;

/// Target frame interval, about 60 fps.
///
/// This caps redraws, not playback: at the top speed a single frame applies
/// several hundred steps, which is what makes a large run resolve smoothly
/// instead of one operation per frame.
const FRAME: Duration = Duration::from_millis(16);

const HELP: &str = "\
algoscope - visualize algorithms in the terminal

USAGE:
    algoscope [OPTIONS]

OPTIONS:
    -m, --mode <MODE>       sort or grid [default: sort]
    -a, --algorithm <NAME>  sort: bubble, insertion, selection
                            grid: bfs, dfs, dijkstra, astar
    -n, --size <N>          elements, in sort mode [default: 48]
    -g, --grid <WxH>        map size, in grid mode [default: fits the terminal]
    -s, --seed <N>          seed the generator, for a reproducible run
        --ascii             ASCII glyphs and no colour
    -h, --help              print this message
    -V, --version           print the version

KEYS:
    space  play/pause      left/right  step        up/down  speed
    tab    next algorithm  r           new input   m        switch mode
    e      edit terrain    ?           all keys    q        quit

Changing algorithm keeps the same input, so the counts of two algorithms
can be compared directly.
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
    // would abandon the user's terminal with no echo and no cursor.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, options);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, options: Options) -> io::Result<()> {
    // A grid the user did not size is sized to the terminal, once. It is not
    // resized afterwards: the trace being replayed was computed for this map,
    // so growing the window mid-run would invalidate it.
    let (grid_width, grid_height) = match options.grid {
        Some(dimensions) => dimensions,
        None => fit_grid(terminal.size()?),
    };

    let mut app = App::new(Config {
        mode: options.mode,
        sort_algorithm: options.sort_algorithm,
        grid_algorithm: options.grid_algorithm,
        size: options.size,
        grid_width,
        grid_height,
        seed: options.seed,
    });
    let theme = options.theme;
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
                // up on its own. The data is never resized to match.
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

/// Choose a grid that fills the terminal, allowing for the chrome.
fn fit_grid(size: Size) -> (u16, u16) {
    // Mirrors the layout: the sidebar is only drawn when there is room, and
    // two rows go to the scrubber and the status line.
    let sidebar = if size.width >= 76 { 32 } else { 0 };
    let usable = Rect::new(0, 0, size.width.saturating_sub(sidebar), size.height.saturating_sub(2));
    let (columns, rows) = GridView::capacity(usable);
    (
        columns.clamp(MIN_GRID, MAX_GRID),
        rows.clamp(MIN_GRID, MAX_GRID),
    )
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    sort_algorithm: sort::Algorithm,
    grid_algorithm: search_algo::Algorithm,
    size: usize,
    /// `None` means fit the terminal.
    grid: Option<(u16, u16)>,
    seed: u64,
    theme: Theme,
}

impl Options {
    /// Parse arguments, or return `Ok(None)` if the program should just exit.
    ///
    /// Hand-rolled to keep the dependency tree at three crates; the surface is
    /// a handful of flags and does not justify an argument parser.
    fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<Option<Self>, String> {
        let mut mode = Mode::default();
        let mut sort_algorithm = sort::Algorithm::default();
        let mut grid_algorithm = search_algo::Algorithm::default();
        let mut size = DEFAULT_SIZE;
        let mut grid = None;
        let mut ascii = false;
        // Resolved after parsing, since which list it names depends on --mode
        // however the two flags happen to be ordered.
        let mut algorithm: Option<String> = None;
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
                "-m" | "--mode" => {
                    mode = parse_mode(&next_value(&mut args, &arg)?)?;
                }
                "-a" | "--algorithm" => {
                    algorithm = Some(next_value(&mut args, &arg)?);
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
                "-g" | "--grid" => {
                    grid = Some(parse_grid(&next_value(&mut args, &arg)?)?);
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

        if let Some(name) = algorithm {
            match mode {
                Mode::Sort => sort_algorithm = parse_sort_algorithm(&name)?,
                Mode::Grid => grid_algorithm = parse_grid_algorithm(&name)?,
            }
        }

        let theme = if ascii { Theme::minimal() } else { Theme::detect() };

        Ok(Some(Self {
            mode,
            sort_algorithm,
            grid_algorithm,
            size,
            grid,
            seed,
            theme,
        }))
    }
}

fn next_value<I: Iterator<Item = String>>(args: &mut I, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_mode(name: &str) -> Result<Mode, String> {
    let lowered = name.to_ascii_lowercase();
    match lowered.as_str() {
        "sort" | "sorting" | "s" => Ok(Mode::Sort),
        "grid" | "path" | "pathfinding" | "g" => Ok(Mode::Grid),
        _ => Err(format!("unknown mode '{name}' (try sort or grid)")),
    }
}

fn parse_grid(value: &str) -> Result<(u16, u16), String> {
    let (width, height) = value
        .split_once(['x', 'X', ','])
        .ok_or_else(|| format!("--grid wants WxH, got '{value}'"))?;
    let parse = |part: &str, which: &str| -> Result<u16, String> {
        let n: u16 = part
            .trim()
            .parse()
            .map_err(|_| format!("--grid {which} wants a number, got '{part}'"))?;
        if !(MIN_GRID..=MAX_GRID).contains(&n) {
            return Err(format!(
                "--grid {which} must be between {MIN_GRID} and {MAX_GRID}, got {n}"
            ));
        }
        Ok(n)
    };
    Ok((parse(width, "width")?, parse(height, "height")?))
}

fn parse_sort_algorithm(name: &str) -> Result<sort::Algorithm, String> {
    // Prefix matching, so `-a ins` works.
    let lowered = name.to_ascii_lowercase();
    sort::Algorithm::ALL
        .into_iter()
        .find(|a| !lowered.is_empty() && a.name().to_ascii_lowercase().starts_with(&lowered))
        .ok_or_else(|| {
            format!("unknown sort '{name}' (try bubble, insertion or selection)")
        })
}

fn parse_grid_algorithm(name: &str) -> Result<search_algo::Algorithm, String> {
    let lowered = name.to_ascii_lowercase();
    if lowered.is_empty() {
        return Err("--algorithm needs a name".to_string());
    }
    // The display names do not match what anyone types, so the aliases are
    // spelled out rather than prefix-matched against them.
    let table = [
        ("bfs", search_algo::Algorithm::Bfs),
        ("breadth", search_algo::Algorithm::Bfs),
        ("dfs", search_algo::Algorithm::Dfs),
        ("depth", search_algo::Algorithm::Dfs),
        ("dijkstra", search_algo::Algorithm::Dijkstra),
        ("astar", search_algo::Algorithm::AStar),
        ("a*", search_algo::Algorithm::AStar),
    ];
    table
        .into_iter()
        .find(|(alias, _)| alias.starts_with(&lowered))
        .map(|(_, algorithm)| algorithm)
        .ok_or_else(|| {
            format!("unknown search '{name}' (try bfs, dfs, dijkstra or astar)")
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
        assert_eq!(options.mode, Mode::Sort);
        assert_eq!(options.sort_algorithm, sort::Algorithm::Bubble);
        assert_eq!(options.size, DEFAULT_SIZE);
        assert!(options.grid.is_none(), "the grid should fit the terminal");
    }

    #[test]
    fn flags_are_read() {
        let options = parse(&["--algorithm", "selection", "--size", "100", "--seed", "42"])
            .unwrap()
            .unwrap();
        assert_eq!(options.sort_algorithm, sort::Algorithm::Selection);
        assert_eq!(options.size, 100);
        assert_eq!(options.seed, 42);
    }

    #[test]
    fn short_flags_match_long_ones() {
        let long = parse(&["--mode", "grid", "--algorithm", "dijkstra", "--grid", "30x20"])
            .unwrap()
            .unwrap();
        let short = parse(&["-m", "grid", "-a", "dijkstra", "-g", "30x20"])
            .unwrap()
            .unwrap();
        assert_eq!(long.grid_algorithm, short.grid_algorithm);
        assert_eq!(long.grid, short.grid);
    }

    #[test]
    fn algorithm_resolves_against_the_selected_mode() {
        let sorting = parse(&["-a", "ins"]).unwrap().unwrap();
        assert_eq!(sorting.sort_algorithm, sort::Algorithm::Insertion);

        let pathfinding = parse(&["-m", "grid", "-a", "astar"]).unwrap().unwrap();
        assert_eq!(pathfinding.grid_algorithm, search_algo::Algorithm::AStar);

        // A sort name is not a valid search, and vice versa.
        assert!(parse(&["-m", "grid", "-a", "bubble"]).is_err());
        assert!(parse(&["-a", "dijkstra"]).is_err());
    }

    #[test]
    fn mode_order_does_not_matter() {
        // --algorithm is resolved after parsing, so it works before --mode too.
        let before = parse(&["-a", "dfs", "-m", "grid"]).unwrap().unwrap();
        let after = parse(&["-m", "grid", "-a", "dfs"]).unwrap().unwrap();
        assert_eq!(before.grid_algorithm, search_algo::Algorithm::Dfs);
        assert_eq!(before.grid_algorithm, after.grid_algorithm);
    }

    #[test]
    fn sort_names_can_be_abbreviated() {
        for (input, want) in [
            ("bubble", sort::Algorithm::Bubble),
            ("BUBBLE", sort::Algorithm::Bubble),
            ("ins", sort::Algorithm::Insertion),
            ("s", sort::Algorithm::Selection),
        ] {
            assert_eq!(parse_sort_algorithm(input).unwrap(), want, "for {input}");
        }
        assert!(parse_sort_algorithm("quick").is_err());
        assert!(parse_sort_algorithm("").is_err());
    }

    #[test]
    fn search_names_accept_the_usual_aliases() {
        for (input, want) in [
            ("bfs", search_algo::Algorithm::Bfs),
            ("breadth", search_algo::Algorithm::Bfs),
            ("dij", search_algo::Algorithm::Dijkstra),
            ("a*", search_algo::Algorithm::AStar),
            ("ASTAR", search_algo::Algorithm::AStar),
        ] {
            assert_eq!(parse_grid_algorithm(input).unwrap(), want, "for {input}");
        }
        assert!(parse_grid_algorithm("quick").is_err());
        assert!(parse_grid_algorithm("").is_err());
    }

    #[test]
    fn grid_dimensions_parse_and_validate() {
        assert_eq!(parse_grid("40x20").unwrap(), (40, 20));
        assert_eq!(parse_grid("40X20").unwrap(), (40, 20));
        assert_eq!(parse_grid("40,20").unwrap(), (40, 20));
        assert_eq!(parse_grid(" 40 x 20 ").unwrap(), (40, 20));

        assert!(parse_grid("40").is_err(), "needs both dimensions");
        assert!(parse_grid("40x").is_err());
        assert!(parse_grid("0x20").is_err(), "below the minimum");
        assert!(parse_grid("9999x20").is_err(), "above the maximum");
        assert!(parse_grid("wide x tall").is_err());
    }

    #[test]
    fn fit_grid_leaves_room_for_the_chrome() {
        // 120 wide: 32 to the sidebar, 88 left, two columns per cell.
        let (columns, rows) = fit_grid(Size::new(120, 30));
        assert_eq!(columns, 44);
        assert_eq!(rows, 28);

        // Narrow enough that the sidebar is dropped, so the full width is the
        // grid's.
        let (columns, _) = fit_grid(Size::new(60, 20));
        assert_eq!(columns, 30);
    }

    #[test]
    fn fit_grid_clamps_tiny_and_huge_terminals() {
        let (columns, rows) = fit_grid(Size::new(4, 3));
        assert!(columns >= MIN_GRID && rows >= MIN_GRID);

        let (columns, rows) = fit_grid(Size::new(2000, 1000));
        assert!(columns <= MAX_GRID && rows <= MAX_GRID);
    }

    #[test]
    fn seed_defaults_to_something_that_varies() {
        // Pinning the seed is opt-in; the default should not be a constant, or
        // every launch would show the identical input.
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
            vec!["--mode", "sideways"],
            vec!["--algorithm"],
            vec!["--size"],
            vec!["--grid"],
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
