//! Turning commits with parents into the picture the branches make.
//!
//! The characters are `git log --graph`'s own — `*` a commit, `|` a line carrying on, `/` and
//! `\` a line changing lane, `-` a line crossing to reach one — and they are ASCII on purpose.
//! The box-drawing and braille that make a prettier graph elsewhere need a font that has them
//! and a terminal that spaces them the way the font meant; over `ssh` to a machine whose console
//! is whatever was there, they come out as boxes or as gaps that put every lane half a column
//! out. A graph that is wrong about which line joins which is worse than no graph, and these six
//! characters are on every terminal that exists.
//!
//! The layout is a lane assignment, done in one pass from the newest commit down. It is a pure
//! function of the list, with no access to the repository and no drawing in it: what the panel
//! shows can then be checked by reading the art back, and the awkward shapes — an octopus merge,
//! a branch with no common ancestor, a graph cut off at its limit with parents that never
//! arrive — are cases in a test file rather than repositories somebody has to build.
//!
//! **Lanes are never shuffled left.** A lane that empties stays empty until something needs a new
//! one, which reuses it. `git log --graph` compacts instead, and gets a narrower drawing at the
//! cost of a diagonal every time a lane closes anywhere to the left. In a window that is also
//! holding an editor, lines that stay in their column are what makes the shape readable at a
//! glance — the diagonals that are left are the ones that mean something: a branch starting, and
//! a branch being merged.

use crate::git::GraphCommit;

/// One character of the drawing, and which lane it belongs to.
///
/// The lane travels with the character because that is what the colour is chosen from: two lines
/// side by side in the same colour are one line as far as the eye is concerned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Glyph {
    pub ch: char,
    pub lane: usize,
}

/// One drawn row: either a commit, or the links that get to the next one.
#[derive(Clone, Debug)]
pub struct Row {
    pub glyphs: Vec<Glyph>,
    /// Which commit of the input this row draws, and `None` for a row that is only lines.
    pub commit: Option<usize>,
}

impl Row {
    /// The row as text. What the drawing does is a property worth asserting about, and a string
    /// is the only form of it a test can read.
    pub fn art(&self) -> String {
        self.glyphs.iter().map(|g| g.ch).collect()
    }

    /// The lane the commit sits in, taken from the `*` itself rather than kept beside it. One
    /// place that knows, so the drawing and the answer cannot come apart.
    ///
    /// Only the tests ask: the drawing carries the lane on every glyph, which is what the colour
    /// is chosen from. A field beside the row would be a second copy of an answer the row
    /// already contains, and the first thing to go stale.
    #[cfg(test)]
    pub fn lane(&self) -> Option<usize> {
        self.glyphs.iter().find(|g| g.ch == '*').map(|g| g.lane)
    }
}

/// Where a lane sits, in characters. One column of art per lane and one of gap between them:
/// the gap is where a diagonal goes, which is why it is there at all.
fn column(lane: usize) -> usize {
    lane * 2
}

/// Lays the commits out into rows.
///
/// The one thing this needs of the input is that no commit comes before one of its children,
/// which is what `--date-order` and `--topo-order` both promise. Given that, a lane is opened
/// when a commit nobody claimed is met, and closed when the last child of a commit hands it on.
pub fn lay_out(commits: &[GraphCommit]) -> Vec<Row> {
    // What each lane is waiting to draw. `None` is a lane that has emptied and can be reused.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();

    for (index, commit) in commits.iter().enumerate() {
        let waiting = |lanes: &Vec<Option<String>>, hash: &str| -> Vec<usize> {
            lanes
                .iter()
                .enumerate()
                .filter(|(_, lane)| lane.as_deref() == Some(hash))
                .map(|(at, _)| at)
                .collect()
        };

        let claimed = waiting(&lanes, &commit.hash);
        // A commit no lane is waiting for is the tip of a branch: it gets a lane of its own,
        // reusing an emptied one rather than widening the drawing.
        let col = match claimed.first() {
            Some(&first) => first,
            None => {
                let free = lanes.iter().position(Option::is_none);
                match free {
                    Some(at) => at,
                    None => {
                        lanes.push(None);
                        lanes.len() - 1
                    }
                }
            }
        };
        lanes[col] = Some(commit.hash.clone());

        // Every other lane waiting for this commit is a branch that starts here. They fold into
        // `col` on a row of their own, above the commit — which is where the eye expects the
        // join, because reading downwards is reading backwards in time.
        let folding: Vec<usize> = claimed.into_iter().filter(|&at| at != col).collect();
        if !folding.is_empty() {
            rows.push(link_row(&lanes, col, &folding, Direction::Into));
            for &at in &folding {
                lanes[at] = None;
            }
        }

        rows.push(commit_row(&lanes, col, index));

        // The first parent carries on in this lane; every other one is a merge, and goes to the
        // lane already waiting for it or to a new one.
        let mut branching: Vec<usize> = Vec::new();
        lanes[col] = commit.parents.first().cloned();
        for parent in commit.parents.iter().skip(1) {
            let at = match waiting(&lanes, parent).first() {
                Some(&at) => at,
                None => {
                    let free = lanes.iter().position(Option::is_none);
                    match free {
                        Some(at) => at,
                        None => {
                            lanes.push(None);
                            lanes.len() - 1
                        }
                    }
                }
            };
            lanes[at] = Some(parent.clone());
            branching.push(at);
        }
        if !branching.is_empty() {
            rows.push(link_row(&lanes, col, &branching, Direction::OutOf));
        }

        // Lanes that emptied at the right edge are dropped so the drawing gives the width back.
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }
    }
    rows
}

/// Which way a link row's diagonals are read: lines arriving at `col` from lanes that end here,
/// or leaving `col` for lanes that start here.
#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Into,
    OutOf,
}

fn commit_row(lanes: &[Option<String>], col: usize, index: usize) -> Row {
    let mut cells = blank(lanes.len());
    for (at, lane) in lanes.iter().enumerate() {
        if lane.is_some() {
            cells[column(at)] = Some(Glyph { ch: '|', lane: at });
        }
    }
    cells[column(col)] = Some(Glyph { ch: '*', lane: col });
    Row { glyphs: trim(cells), commit: Some(index) }
}

/// A row of nothing but lines: the ones going straight down, and the ones changing lane.
fn link_row(lanes: &[Option<String>], col: usize, moving: &[usize], direction: Direction) -> Row {
    let mut cells = blank(lanes.len());
    for (at, lane) in lanes.iter().enumerate() {
        // A lane that is changing column draws its diagonal instead of a bar: drawing both would
        // put a line in the column it is in the middle of leaving.
        if lane.is_some() && !moving.contains(&at) {
            cells[column(at)] = Some(Glyph { ch: '|', lane: at });
        }
    }
    // `col` itself always carries on downwards: it is the commit's own lane, and it has either
    // just been drawn above or is about to be.
    cells[column(col)] = Some(Glyph { ch: '|', lane: col });

    for &at in moving {
        if at == col {
            continue;
        }
        // Read downwards. A lane to the right of the commit leans left to reach it and draws
        // `/`; one to the left leans right and draws `\`. Leaving is the mirror of arriving,
        // which is why the same two characters serve both and the direction only decides which.
        let (edge, ch) = if at > col {
            (column(at) - 1, if direction == Direction::Into { '/' } else { '\\' })
        } else {
            (column(at) + 1, if direction == Direction::Into { '\\' } else { '/' })
        };
        // The run between the two lanes, for a jump of more than one. It goes under the bars it
        // passes rather than through them: a lane it crosses keeps its `|`, so a line that ends
        // three lanes away still reads as one line and not as a join to everything on the way.
        let (from, to) = if at > col { (column(col) + 1, edge) } else { (edge + 1, column(col)) };
        for cell in cells.iter_mut().take(to).skip(from) {
            if cell.is_none() {
                *cell = Some(Glyph { ch: '-', lane: at });
            }
        }
        cells[edge] = Some(Glyph { ch, lane: at });
    }
    Row { glyphs: trim(cells), commit: None }
}

fn blank(lanes: usize) -> Vec<Option<Glyph>> {
    vec![None; column(lanes.max(1))]
}

/// Fills the gaps with spaces and drops the ones off the right-hand end, so a row is as wide as
/// it needs to be and no wider.
fn trim(cells: Vec<Option<Glyph>>) -> Vec<Glyph> {
    let last = cells.iter().rposition(Option::is_some);
    let Some(last) = last else { return Vec::new() };
    cells
        .into_iter()
        .take(last + 1)
        .map(|cell| cell.unwrap_or(Glyph { ch: ' ', lane: 0 }))
        .collect()
}

/// How wide the widest row is, so the column that holds the art can be sized once for all of
/// them. A graph whose text starts at a different column on every line is a graph nobody reads.
pub fn width(rows: &[Row]) -> usize {
    rows.iter().map(|r| r.glyphs.len()).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GraphCommit;

    fn commit(hash: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit {
            hash: hash.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            refs: Vec::new(),
            author: "someone".to_string(),
            when: "now".to_string(),
            subject: hash.to_string(),
        }
    }

    /// The art as one block of text, which is the only way to look at a drawing and say whether
    /// it is the right one.
    fn drawing(commits: &[GraphCommit]) -> String {
        lay_out(commits).iter().map(Row::art).collect::<Vec<_>>().join("\n")
    }


    #[test]
    fn a_line_of_commits_is_one_lane() {
        let commits = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        assert_eq!(drawing(&commits), "*\n*\n*");
    }

    /// The shape everything else is built out of: a branch leaves, does something, and comes
    /// back. Both diagonals have to be there and they have to lean opposite ways, or the picture
    /// says the branch started where it ended.
    #[test]
    fn a_branch_and_its_merge() {
        let commits = [
            commit("m", &["a", "b"]),
            commit("a", &["c"]),
            commit("b", &["c"]),
            commit("c", &[]),
        ];
        assert_eq!(drawing(&commits), ["*", "|\\", "* |", "| *", "|/", "*"].join("\n"));
    }

    /// A merge of three, which git calls an octopus. Rare, and the reason the merge side is a
    /// loop rather than a special case for the second parent.
    #[test]
    fn an_octopus_merge_opens_a_lane_for_every_parent() {
        let commits = [
            commit("m", &["a", "b", "c"]),
            commit("a", &["r"]),
            commit("b", &["r"]),
            commit("c", &["r"]),
            commit("r", &[]),
        ];
        let rows = lay_out(&commits);
        // Three lanes leave the merge: its own and two more. `git log --graph` writes this row
        // as `|\ \`, with a gap where the second parent's line reaches across. The gap is where
        // it went, so it is drawn: the run of `-` says the line came from the commit rather than
        // from wherever else is in that column.
        assert_eq!(rows[1].art(), "|\\-\\");
        assert_eq!(rows[2].art(), "* | |");
    }

    /// Two roots with nothing in common — an imported history, or a `gh-pages` branch. Neither
    /// lane may reach across to the other, since there is no link between them to draw.
    #[test]
    fn unrelated_histories_never_touch() {
        let commits = [commit("b", &[]), commit("a", &[])];
        assert_eq!(drawing(&commits), "*\n*");
    }

    /// A lane that has emptied is used again by the next branch instead of the drawing getting
    /// wider. Fifty commits into a repository where branches come and go, the difference is a
    /// graph three columns wide and one that is thirty.
    #[test]
    fn an_emptied_lane_is_reused() {
        let commits = [
            commit("m", &["a", "b"]),
            commit("a", &["c"]),
            commit("b", &["c"]),
            commit("c", &["d"]),
            // A tip nobody has merged, met after the branch above has closed: it takes the lane
            // that one gave back.
            commit("z", &["d"]),
            commit("d", &[]),
        ];
        let rows = lay_out(&commits);
        let z = rows.iter().find(|r| r.commit == Some(4)).expect("z is drawn");
        assert_eq!(z.lane(), Some(1), "z should reuse the lane the merged branch gave back");
    }

    /// The graph is cut off at a limit, so the oldest commits on screen have parents that are
    /// never drawn. Their lanes stay open at the bottom rather than closing on a commit that is
    /// not there — which is the truth: the history carries on past the window.
    #[test]
    fn parents_that_never_arrive_leave_their_lanes_open() {
        let commits = [commit("b", &["nowhere"]), commit("a", &["also-nowhere"])];
        assert_eq!(drawing(&commits), "*\n| *");
    }

    /// Every glyph carries the lane it belongs to, because the colour is picked from it. A
    /// diagonal belongs to the line that is moving, not to the lane it is passing through.
    #[test]
    fn a_diagonal_is_coloured_as_the_line_that_moves() {
        let commits = [
            commit("m", &["a", "b"]),
            commit("a", &["c"]),
            commit("b", &["c"]),
            commit("c", &[]),
        ];
        let rows = lay_out(&commits);
        let opening = &rows[1];
        assert_eq!(opening.art(), "|\\");
        assert_eq!(opening.glyphs[0].lane, 0);
        assert_eq!(opening.glyphs[1].lane, 1, "the diagonal belongs to the lane it opens");
    }

    /// A line reaching a lane three across passes *under* the ones in between: a lane it
    /// crosses keeps its own `|`, and only the gaps between lanes get the `-`. Without that rule
    /// a merge from one side of the graph to the other would draw itself straight through two
    /// unrelated branches and read as joining all three.
    #[test]
    fn a_long_reach_passes_under_the_lanes_it_crosses() {
        let commits = [
            commit("x", &["p"]),
            commit("y", &["q"]),
            commit("z", &["r"]),
            // Merges `p`, which is three lanes away and still live.
            commit("w", &["s", "p"]),
        ];
        let rows = lay_out(&commits);
        let reach = rows.last().expect("the merge draws a link row");
        assert_eq!(reach.art(), " /|-|-|");
        // The two bars are the branches being crossed, and they are still their own lanes.
        assert_eq!(reach.glyphs[2].lane, 1);
        assert_eq!(reach.glyphs[4].lane, 2);
        // The line that is moving owns its diagonal and the run that gets there.
        assert_eq!(reach.glyphs[1].ch, '/');
        assert_eq!(reach.glyphs[3].ch, '-');
    }

    /// Three branches folding into one lane on the same row. They overlap, because there is only
    /// one row to draw them on and they all arrive at the same place — which is what the drawing
    /// then says: one line, joined three times.
    #[test]
    fn several_branches_can_fold_on_one_row() {
        let commits = [
            commit("head", &["x"]),
            commit("one", &["x"]),
            commit("two", &["x"]),
            commit("three", &["x"]),
            commit("x", &[]),
        ];
        let rows = lay_out(&commits);
        let fold = rows.iter().find(|r| r.art().contains('/')).expect("a fold is drawn");
        assert_eq!(fold.art(), "|/-/-/");
    }
}
