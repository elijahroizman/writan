//! Spelling for the documents Writan draws itself (Word documents and presentations), whose
//! text the web view never sees: the page asks which words are misspelled, what to suggest, and
//! to learn a word. Each system's own checker, in the language the user has set:
//! NSSpellChecker on a Mac (with the words learned in other apps), the Windows spell checker,
//! and the Hunspell dictionaries installed on Linux (learned words kept in Writan's data folder).

use tauri::{AppHandle, Runtime};

/// The words in `words` that are misspelled.
#[tauri::command]
pub async fn spell_check<R: Runtime>(app: AppHandle<R>, words: Vec<String>) -> Result<Vec<String>, String> {
    run(app, move |s| s.misspelled(&words)).await
}

/// Corrections for a misspelled word, best first.
#[tauri::command]
pub async fn spell_suggest<R: Runtime>(app: AppHandle<R>, word: String) -> Result<Vec<String>, String> {
    run(app, move |s| s.suggest(&word)).await
}

/// Adds a word to the user's dictionary.
#[tauri::command]
pub async fn spell_learn<R: Runtime>(app: AppHandle<R>, word: String) -> Result<(), String> {
    run(app, move |s| s.learn(&word)).await
}

trait Speller {
    fn misspelled(&mut self, words: &[String]) -> Vec<String>;
    fn suggest(&mut self, word: &str) -> Vec<String>;
    fn learn(&mut self, word: &str);
}

// ---- macOS: NSSpellChecker, on the main thread like the rest of AppKit.

#[cfg(target_os = "macos")]
async fn run<R: Runtime, T: Send + Default + 'static>(app: AppHandle<R>, f: impl FnOnce(&mut dyn Speller) -> T + Send + 'static) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&mut mac::Mac));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or_default()).await.map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSNotFound, NSRange, NSString};

    pub struct Mac;

    impl super::Speller for Mac {
        fn misspelled(&mut self, words: &[String]) -> Vec<String> {
            let checker = NSSpellChecker::sharedSpellChecker();
            words
                .iter()
                .filter(|w| {
                    let range = checker.checkSpellingOfString_startingAt(&NSString::from_str(w), 0);
                    range.location != NSNotFound as usize && range.length > 0
                })
                .cloned()
                .collect()
        }

        fn suggest(&mut self, word: &str) -> Vec<String> {
            let checker = NSSpellChecker::sharedSpellChecker();
            let text = NSString::from_str(word);
            let range = NSRange::new(0, text.length());
            checker
                .guessesForWordRange_inString_language_inSpellDocumentWithTag(range, &text, None, 0)
                .map(|guesses| guesses.iter().map(|g| g.to_string()).collect())
                .unwrap_or_default()
        }

        fn learn(&mut self, word: &str) {
            NSSpellChecker::sharedSpellChecker().learnWord(&NSString::from_str(word));
        }
    }
}

// ---- Windows: the system spell checker (ISpellChecker), in the user's language.

#[cfg(windows)]
async fn run<R: Runtime, T: Send + Default + 'static>(_app: AppHandle<R>, f: impl FnOnce(&mut dyn Speller) -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || match win::Win::new() {
        Some(mut s) => f(&mut s),
        None => T::default(),
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
mod win {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Globalization::{GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

    pub struct Win(ISpellChecker);

    impl Win {
        /// A checker for the user's language, or English when Windows has none for it.
        pub fn new() -> Option<Self> {
            unsafe {
                // Already initialised on this thread is fine.
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let factory: ISpellCheckerFactory = CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
                let mut name = [0u16; 85];
                let n = GetUserDefaultLocaleName(&mut name);
                let user = if n > 1 { String::from_utf16_lossy(&name[..n as usize - 1]) } else { String::new() };
                for tag in [user.as_str(), "en-US"] {
                    if tag.is_empty() {
                        continue;
                    }
                    let tag = HSTRING::from(tag);
                    if factory.IsSupported(&tag).map(|b| b.as_bool()).unwrap_or(false) {
                        if let Ok(checker) = factory.CreateSpellChecker(&tag) {
                            return Some(Win(checker));
                        }
                    }
                }
                None
            }
        }
    }

    impl super::Speller for Win {
        fn misspelled(&mut self, words: &[String]) -> Vec<String> {
            words
                .iter()
                .filter(|w| unsafe {
                    let Ok(errors) = self.0.Check(&HSTRING::from(w.as_str())) else { return false };
                    let mut error = None;
                    errors.Next(&mut error).is_ok() && error.is_some()
                })
                .cloned()
                .collect()
        }

        fn suggest(&mut self, word: &str) -> Vec<String> {
            let mut out = Vec::new();
            unsafe {
                let Ok(list) = self.0.Suggest(&HSTRING::from(word)) else { return out };
                loop {
                    let mut item = [PWSTR::null()];
                    let mut fetched = 0u32;
                    if list.Next(&mut item, Some(&mut fetched as *mut u32)).is_err() || fetched == 0 {
                        break;
                    }
                    if let Ok(s) = item[0].to_string() {
                        out.push(s);
                    }
                    CoTaskMemFree(Some(item[0].0 as *const _));
                }
            }
            out
        }

        fn learn(&mut self, word: &str) {
            unsafe {
                let _ = self.0.Add(&HSTRING::from(word));
            }
        }
    }
}

// ---- Linux: Hunspell dictionaries from the system (hunspell-en-us and the like).

#[cfg(target_os = "linux")]
async fn run<R: Runtime, T: Send + Default + 'static>(app: AppHandle<R>, f: impl FnOnce(&mut dyn Speller) -> T + Send + 'static) -> Result<T, String> {
    use tauri::Manager;
    let words = app.path().app_data_dir().ok().map(|d| d.join("spelling-words.txt"));
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = linux::SPELLER.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(linux::Hunspell::load(words));
        }
        match guard.as_mut() {
            Some(s) => f(s),
            None => T::default(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    pub static SPELLER: Mutex<Option<Hunspell>> = Mutex::new(None);

    pub struct Hunspell {
        dict: Option<spellbook::Dictionary>,
        /// Where learned words are kept, one per line.
        words: Option<PathBuf>,
    }

    const DIRS: &[&str] = &["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/app/share/hunspell"];

    /// The user's language, as Hunspell names dictionaries: `en_AU` from `en_AU.UTF-8`.
    pub fn language() -> Option<String> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
            .map(|v| v.split(['.', '@']).next().unwrap_or("").to_string())
    }

    /// The dictionary for the user's language, else another of the same language, else US English.
    pub fn find_dictionary(dirs: &[&Path], language: Option<&str>) -> Option<(PathBuf, PathBuf)> {
        let lang = language.unwrap_or("en_US");
        let base = lang.split('_').next().unwrap_or("en");
        let pair = |dir: &Path, name: &str| {
            let (aff, dic) = (dir.join(format!("{name}.aff")), dir.join(format!("{name}.dic")));
            (aff.is_file() && dic.is_file()).then_some((aff, dic))
        };
        for name in [lang.to_string(), lang.replace('-', "_")] {
            if let Some(found) = dirs.iter().find_map(|d| pair(d, &name)) {
                return Some(found);
            }
        }
        let mut others: Vec<String> = dirs
            .iter()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".dic").map(str::to_string))
            .filter(|n| n == base || n.starts_with(&format!("{base}_")))
            .collect();
        others.sort();
        others.push("en_US".to_string());
        others.iter().find_map(|n| dirs.iter().find_map(|d| pair(d, n)))
    }

    impl Hunspell {
        pub fn load(words: Option<PathBuf>) -> Self {
            let dirs: Vec<&Path> = DIRS.iter().map(Path::new).collect();
            let dict = find_dictionary(&dirs, language().as_deref()).and_then(|(aff, dic)| {
                let aff = std::fs::read_to_string(aff).ok()?;
                let dic = std::fs::read_to_string(dic).ok()?;
                spellbook::Dictionary::new(&aff, &dic).ok()
            });
            let mut s = Hunspell { dict, words };
            if let (Some(dict), Some(path)) = (&mut s.dict, &s.words) {
                for w in std::fs::read_to_string(path).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()) {
                    let _ = dict.add(w.trim());
                }
            }
            s
        }

        #[cfg(test)]
        pub fn from_files(aff: &str, dic: &str) -> Self {
            Hunspell { dict: spellbook::Dictionary::new(aff, dic).ok(), words: None }
        }
    }

    impl super::Speller for Hunspell {
        fn misspelled(&mut self, words: &[String]) -> Vec<String> {
            let Some(dict) = &self.dict else { return Vec::new() };
            words.iter().filter(|w| !dict.check(w)).cloned().collect()
        }

        fn suggest(&mut self, word: &str) -> Vec<String> {
            let mut out = Vec::new();
            if let Some(dict) = &self.dict {
                dict.suggest(word, &mut out);
            }
            out
        }

        fn learn(&mut self, word: &str) {
            if let Some(dict) = &mut self.dict {
                let _ = dict.add(word);
            }
            if let Some(path) = &self.words {
                use std::io::Write;
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{word}");
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::Speller;
        use super::*;

        #[test]
        fn finds_the_users_language_or_falls_back() {
            let dir = tempfile::tempdir().unwrap();
            for n in ["en_US", "en_GB", "de_DE"] {
                std::fs::write(dir.path().join(format!("{n}.aff")), "").unwrap();
                std::fs::write(dir.path().join(format!("{n}.dic")), "0\n").unwrap();
            }
            let dirs = [dir.path()];
            let name = |l: Option<&str>| find_dictionary(&dirs, l).map(|(a, _)| a.file_stem().unwrap().to_string_lossy().into_owned());
            assert_eq!(name(Some("de_DE")).as_deref(), Some("de_DE"));
            // No Australian dictionary: another English one.
            assert_eq!(name(Some("en_AU")).as_deref(), Some("en_GB"));
            assert_eq!(name(Some("fr_FR")).as_deref(), Some("en_US"));
            assert_eq!(name(None).as_deref(), Some("en_US"));
        }

        #[test]
        fn checks_suggests_and_learns_with_the_system_dictionary() {
            let dirs: Vec<&Path> = DIRS.iter().map(Path::new).collect();
            let Some((aff, dic)) = find_dictionary(&dirs, Some("en_US")) else {
                eprintln!("no Hunspell dictionary installed; skipped");
                return;
            };
            let mut s = Hunspell::from_files(&std::fs::read_to_string(aff).unwrap(), &std::fs::read_to_string(dic).unwrap());
            let words: Vec<String> = ["The", "quick", "brwon", "fox", "doesn't", "jumpped"].iter().map(|w| w.to_string()).collect();
            assert_eq!(s.misspelled(&words), ["brwon", "jumpped"]);
            assert!(s.suggest("brwon").iter().any(|w| w == "brown"));
            s.learn("Writan");
            assert!(s.misspelled(&["Writan".to_string()]).is_empty());
        }
    }
}

// ---- Elsewhere: no checker.

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
async fn run<R: Runtime, T: Send + Default + 'static>(_app: AppHandle<R>, _f: impl FnOnce(&mut dyn Speller) -> T + Send + 'static) -> Result<T, String> {
    Ok(T::default())
}
