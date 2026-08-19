//! `clee --install-app`: puts a CleeCode launcher in /Applications, so the editor can be
//! started from the Dock and chosen as the application that opens a file or a folder.
//!
//! The bundle is built here, on the machine it will run on, rather than shipped ready-made.
//! That is what keeps it free: a `.app` downloaded from the internet arrives with the
//! quarantine flag on it and needs an Apple Developer signature (and the yearly fee) or the
//! user has to go and unblock it by hand, while a bundle written locally by a program the
//! user just ran carries no quarantine at all and simply opens. It is also why this is a
//! command and not something a normal launch does: putting an icon in somebody's
//! Applications folder is a decision, like `--install-font` touching their fonts.

/// Builds the launcher. Run via `clee --install-app`.
pub fn install() {
    #[cfg(target_os = "macos")]
    macos::install();
    #[cfg(not(target_os = "macos"))]
    eprintln!(
        "--install-app builds a macOS .app bundle and only works there.\n\
         On Linux the equivalent is a .desktop file; it is not written yet."
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The icon, drawn by assets/icon/make-icon.py and compiled in so that installing needs
    /// nothing but the binary itself — no source tree, no download.
    const ICON: &[u8] = include_bytes!("../assets/icon/CleeCode.icns");
    const ICON_NAME: &str = "cleecode";
    const APP: &str = "CleeCode.app";
    const BUNDLE_ID: &str = "com.msavox.cleecode";
    /// The terminal the launcher opens. CleeCode is a terminal application, so something has
    /// to host it; Ghostty is the one the editor is built against and the one whose pictures,
    /// keyboard reporting and font handling the app expects.
    const TERMINAL: &str = "Ghostty";

    /// The file types the launcher offers itself for. Deliberately a list of content types
    /// rather than the `*` an AppleScript droplet declares by default: CleeCode has no
    /// business appearing in the "Open With" menu of a disk image or a photo, and
    /// `LSHandlerRank = Alternate` (set below) means it offers itself without trying to
    /// take the default away from anything.
    const TEXT_TYPES: &[&str] = &[
        "public.source-code",
        "public.plain-text",
        "public.script",
        "public.shell-script",
        "public.json",
        "public.xml",
        "public.yaml",
        "net.daringfireball.markdown",
    ];

    pub fn install() {
        println!("Building {APP}...");

        let clee = match launcher_target() {
            Some(p) => p,
            None => {
                eprintln!("Could not work out where this copy of clee lives.");
                return;
            }
        };
        let Some(dir) = destination() else { return };
        let app = dir.join(APP);
        if !clear_previous(&app) {
            return;
        }

        let script = std::env::temp_dir().join(format!("cleecode-launcher-{}.applescript", std::process::id()));
        if let Err(e) = std::fs::write(&script, applescript(&clee)) {
            eprintln!("Could not write the launcher script: {e}");
            return;
        }
        let compiled = run("/usr/bin/osacompile", &["-o".as_ref(), app.as_os_str(), script.as_os_str()]);
        let _ = std::fs::remove_file(&script);
        if let Err(e) = compiled {
            eprintln!("osacompile failed: {e}");
            return;
        }

        if let Err(e) = dress_up(&app) {
            eprintln!("Could not finish the bundle: {e}");
            return;
        }

        // osacompile signs the applet as it builds it, and everything above changed the
        // bundle underneath that signature. Signing again, ad hoc, leaves the seal matching
        // what is actually on disk — without it macOS can decide the app is damaged.
        if let Err(e) = run("/usr/bin/codesign", &["--force".as_ref(), "--sign".as_ref(), "-".as_ref(), app.as_os_str()]) {
            eprintln!("Note: could not re-sign the bundle ({e}); it may still work.");
        }
        // Tell LaunchServices about it now, rather than waiting for it to notice: until it
        // has been read, the icon is generic and the app is missing from "Open With".
        let _ = run(LSREGISTER, &["-f".as_ref(), app.as_os_str()]);

        println!("Installed: {}", app.display());
        report(&clee);
    }

    /// Where the app goes: /Applications when the user can write there (the usual case for
    /// an admin account), and their own ~/Applications when they cannot, which is a real
    /// folder macOS treats the same way rather than a consolation prize.
    fn destination() -> Option<PathBuf> {
        let shared = PathBuf::from("/Applications");
        let probe = shared.join(".cleecode-write-test");
        if std::fs::create_dir(&probe).is_ok() {
            let _ = std::fs::remove_dir(&probe);
            return Some(shared);
        }
        let personal = dirs::home_dir()?.join("Applications");
        match std::fs::create_dir_all(&personal) {
            Ok(()) => {
                println!("/Applications is not writable, using {}.", personal.display());
                Some(personal)
            }
            Err(e) => {
                eprintln!("Could not write to /Applications or create {}: {e}", personal.display());
                None
            }
        }
    }

    /// Removes an earlier install so this one can replace it — but only once it has checked
    /// the bundle really is ours. Deleting something else that happens to be called
    /// CleeCode.app because a flag was typed is not a mistake worth being able to make.
    fn clear_previous(app: &Path) -> bool {
        if !app.exists() {
            return true;
        }
        let id = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(app.join("Contents/Info.plist"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        if id.as_deref() != Some(BUNDLE_ID) {
            eprintln!(
                "{} already exists and was not built by CleeCode (bundle id {}).\n\
                 Move it out of the way first; nothing has been changed.",
                app.display(),
                id.as_deref().unwrap_or("unreadable")
            );
            return false;
        }
        match std::fs::remove_dir_all(app) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("Could not replace {}: {e}", app.display());
                false
            }
        }
    }

    /// The path the launcher should call. `current_exe` resolves symlinks, which for a
    /// Homebrew install points inside the Cellar at the version installed today — a path
    /// that stops existing at the next `brew upgrade`. When a stable wrapper on the usual
    /// prefixes leads to the same binary, that is the one to write down instead.
    fn launcher_target() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let real = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
        for stable in ["/opt/homebrew/bin/clee", "/usr/local/bin/clee"] {
            let stable = PathBuf::from(stable);
            if std::fs::canonicalize(&stable).is_ok_and(|p| p == real) {
                return Some(stable);
            }
        }
        Some(real)
    }

    /// The launcher itself. It is AppleScript, and it has to be: a double-clicked document
    /// does not arrive in `argv`, it arrives as an `odoc` Apple Event, which a shell script
    /// in a bundle never sees — so a shell launcher would open an empty editor every time.
    /// `on open` is the handler that receives it.
    ///
    /// A running Ghostty is asked for a window through its scripting dictionary rather than
    /// launched again. `open -n` would work too, and was the obvious way to do it, but it
    /// starts a *second* Ghostty: a second icon in the Dock next to the one already pinned
    /// there, every time a file is opened. `new window with configuration` puts the window
    /// in the Ghostty that is already running, where it belongs.
    ///
    /// That costs one thing: talking to another application is automation, so macOS asks
    /// for permission the first time. If it is refused, the launcher falls back to `open -n`
    /// and still works — with the extra instance it was trying to avoid.
    fn applescript(clee: &Path) -> String {
        let clee = escape(&clee.to_string_lossy());
        format!(
            r#"-- CleeCode launcher, written by `clee --install-app`. Rebuild it with that
-- command after moving clee: the path below is compiled into it.

property cleePath : "{clee}"

on run
	-- Clicked in the Dock with no document: come back to the project last worked on.
	launchClee(missing value)
end run

on open theItems
	repeat with anItem in theItems
		launchClee(POSIX path of anItem)
	end repeat
end open

on launchClee(target)
	if target is missing value then
		set projectRoot to POSIX path of (path to home folder)
		set cleeCommand to quoted form of cleePath & " --resume"
	else
		set t to target
		-- POSIX path of a folder comes with a trailing slash; dirname would then hand back
		-- the folder above it, and the project root would be one level out.
		if t ends with "/" and length of t is greater than 1 then set t to text 1 thru -2 of t
		-- A folder becomes the project root; a file opens with its own folder as the root,
		-- which is what typing `clee thatfile` in that directory would have done.
		set projectRoot to do shell script "t=" & quoted form of t & "; if [ -d \"$t\" ]; then printf %s \"$t\"; else dirname \"$t\"; fi"
		set cleeCommand to quoted form of cleePath & " " & quoted form of t
	end if

	if ghosttyIsRunning() then
		try
			tell application "{TERMINAL}"
				set surface to new surface configuration
				set command of surface to cleeCommand
				set initial working directory of surface to projectRoot
				new window with configuration surface
				activate
			end tell
			return
		on error
			-- Automation refused, or a Ghostty too old to be scripted. Carry on below.
		end try
	end if
	openNewInstance(projectRoot, cleeCommand)
end launchClee

on ghosttyIsRunning()
	-- Asked of the process table rather than of System Events, which would need a
	-- permission of its own just to answer.
	return (do shell script "pgrep -x ghostty > /dev/null 2>&1 && echo yes || echo no") is "yes"
end ghosttyIsRunning

on openNewInstance(projectRoot, cleeCommand)
	try
		do shell script "open -na {TERMINAL} --args --working-directory=" & quoted form of projectRoot & " -e /bin/sh -c " & quoted form of ("exec " & cleeCommand)
	-- Not named `message`: that is the name of a `display alert` parameter, and AppleScript
	-- reads it as one wherever it appears, so the alert below would not compile.
	on error why
		display alert "CleeCode" message "Could not start {TERMINAL}." & return & return & why as critical
	end try
end openNewInstance
"#
        )
    }

    /// Escapes a path for an AppleScript string literal.
    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Everything osacompile does not know to do: the identity, the icon, and the list of
    /// things the launcher is willing to open.
    fn dress_up(app: &Path) -> std::io::Result<()> {
        let plist = app.join("Contents/Info.plist");
        let icon = app.join("Contents/Resources").join(format!("{ICON_NAME}.icns"));
        std::fs::write(icon, ICON)?;

        let doc_types = format!(
            r#"[{{"CFBundleTypeName":"Source or text file","CFBundleTypeRole":"Editor","LSHandlerRank":"Alternate","LSItemContentTypes":[{}]}},
                {{"CFBundleTypeName":"Folder","CFBundleTypeRole":"Editor","LSHandlerRank":"Alternate","LSItemContentTypes":["public.folder"]}}]"#,
            TEXT_TYPES.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(",")
        );
        let version = env!("CARGO_PKG_VERSION");
        let set: [(&str, &str, &str); 9] = [
            ("-replace", "CFBundleIdentifier", BUNDLE_ID),
            ("-replace", "CFBundleName", "CleeCode"),
            ("-replace", "CFBundleDisplayName", "CleeCode"),
            ("-replace", "CFBundleIconFile", ICON_NAME),
            ("-replace", "CFBundleShortVersionString", version),
            ("-replace", "CFBundleVersion", version),
            ("-replace", "LSApplicationCategoryType", "public.app-category.developer-tools"),
            ("-replace", "NSHumanReadableCopyright", "MIT — github.com/msavox/cleecode"),
            // The sentence macOS shows when it asks whether CleeCode may control Ghostty.
            // The applet template's own wording ("This script needs to control other
            // applications to run.") explains nothing to somebody staring at a dialog.
            (
                "-replace",
                "NSAppleEventsUsageDescription",
                "CleeCode opens the editor in a new window of the Ghostty you already have running.",
            ),
        ];
        for (op, key, value) in set {
            let _ = run("/usr/bin/plutil", &[op.as_ref(), key.as_ref(), "-string".as_ref(), value.as_ref(), plist.as_os_str()]);
        }
        let _ = run("/usr/bin/plutil", &["-replace".as_ref(), "CFBundleDocumentTypes".as_ref(), "-json".as_ref(), doc_types.as_ref(), plist.as_os_str()]);
        // Points at an asset catalog the applet template ships and we have replaced; left in
        // place it wins over CFBundleIconFile and the icon stays a blank droplet.
        let _ = run("/usr/bin/plutil", &["-remove".as_ref(), "CFBundleIconName".as_ref(), plist.as_os_str()]);
        // Boilerplate the applet template declares for scripts that drive Music, Photos,
        // HomeKit and the rest. This one asks Ghostty for a window and nothing else, and a
        // launcher that announces it may want the camera is a launcher nobody should trust.
        for key in [
            "NSAppleMusicUsageDescription",
            "NSCalendarsUsageDescription",
            "NSCameraUsageDescription",
            "NSContactsUsageDescription",
            "NSHomeKitUsageDescription",
            "NSMicrophoneUsageDescription",
            "NSPhotoLibraryUsageDescription",
            "NSRemindersUsageDescription",
            "NSSiriUsageDescription",
            "NSSystemAdministrationUsageDescription",
        ] {
            let _ = run("/usr/bin/plutil", &["-remove".as_ref(), key.as_ref(), plist.as_os_str()]);
        }
        Ok(())
    }

    /// What to do with it, and the one thing that could still be missing.
    fn report(clee: &Path) {
        println!("It runs: {}", clee.display());
        if !["/Applications", "~/Applications"]
            .iter()
            .any(|d| Path::new(&d.replace('~', &dirs::home_dir().unwrap_or_default().to_string_lossy())).join(format!("{TERMINAL}.app")).exists())
        {
            println!("\n{TERMINAL} is not installed — the launcher needs it:");
            println!("    brew install --cask ghostty");
        }
        println!(
            "\nTo keep it in the Dock, open it once and choose Options > Keep in Dock.\n\
             To make it the application that opens a kind of file: select one in Finder,\n\
             press Cmd+I, pick CleeCode under \"Open with\", then \"Change All...\".\n\
             \n\
             Uninstall by dragging {APP} to the Bin."
        );
    }

    const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/\
                              LaunchServices.framework/Support/lsregister";

    /// Runs a command, turning "it ran and failed" into an error like "it would not run".
    fn run(program: &str, args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
        let out = Command::new(program).args(args).output()?;
        if out.status.success() {
            return Ok(());
        }
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.trim();
        Err(std::io::Error::other(if why.is_empty() { out.status.to_string() } else { why.to_string() }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The generated script has paths pasted into string literals, so a path containing
        /// a quote has to come out as a quote rather than as the end of the string.
        #[test]
        fn quotes_and_backslashes_survive_the_paste() {
            assert_eq!(escape(r#"/Users/a"b\c/clee"#), r#"/Users/a\"b\\c/clee"#);
        }

        /// The script is only ever compiled at install time, so a typo in it would sit there
        /// unnoticed until somebody ran the command and got a bundle that does nothing. This
        /// compiles it the same way `install` does, into a temporary directory that is thrown
        /// away — nothing is installed, and /Applications is not touched.
        #[test]
        fn the_generated_script_is_valid_applescript() {
            let dir = std::env::temp_dir().join(format!("cleecode-applescript-test-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let src = dir.join("launcher.applescript");
            std::fs::write(&src, applescript(Path::new("/opt/homebrew/bin/clee"))).unwrap();
            let out = dir.join("Test.app");
            let compiled = run("/usr/bin/osacompile", &["-o".as_ref(), out.as_os_str(), src.as_os_str()]);
            let _ = std::fs::remove_dir_all(&dir);
            compiled.expect("osacompile rejected the launcher script");
        }

        /// The whole point of the applet: a double-clicked file arrives at `on open`, and a
        /// click on the icon with no file arrives at `on run` and resumes.
        #[test]
        fn the_launcher_handles_both_ways_of_starting_it() {
            let script = applescript(Path::new("/opt/homebrew/bin/clee"));
            assert!(script.contains("on open theItems"));
            assert!(script.contains("on run"));
            assert!(script.contains("--resume"));
            assert!(script.contains("/opt/homebrew/bin/clee"));
        }
    }
}
