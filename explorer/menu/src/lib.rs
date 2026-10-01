//! The File Explorer context menu, described once.
//!
//! The shell extension asks [`menu`] what to show for a selection and the CLI
//! asks [`action`] what a chosen entry runs, so the two can never disagree.
//! Nothing here touches Windows; the model is plain data and is tested on
//! every platform.

/// The COM class of the context-menu command, without braces.
pub const CLASS_ID: &str = "7A1D3C52-6B0E-4E2F-9C41-5D8E2B7F4A10";
/// [`CLASS_ID`] as the integer the Windows bindings build a GUID from.
pub const CLASS_ID_VALUE: u128 = 0x7A1D3C52_6B0E_4E2F_9C41_5D8E2B7F4A10;
/// The extension library `just install` places beside `just.exe`.
pub const SHELL_LIBRARY: &str = "justtools_shell.dll";

/// A JustTools command the menu can start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Video,
    Mp3,
    Audio,
    Wav,
    Optimize,
    Resize,
    Webp,
    Avif,
    Jpg,
    Png,
    Crop,
    Rmbg,
    Pdf,
    Svg,
    Json,
    Links,
    Zip,
    Paste,
}

const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "m2ts", "3gp", "ogv",
];
const AUDIO: &[&str] = &[
    "aac", "ac3", "aiff", "alac", "ape", "flac", "m4a", "mka", "mp2", "mp3", "ogg", "opus", "wav",
    "wma",
];
const STILL: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "qoi"];
const ALPHA: &[&str] = &["png", "webp", "tif", "tiff", "qoi"];
const WEBP_SOURCES: &[&str] = &["jpg", "jpeg", "png", "bmp", "tif", "tiff"];
const AVIF_SOURCES: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"];
const LINK_SOURCES: &[&str] = &[
    "pdf", "xlsx", "xlsm", "docx", "docm", "pptx", "pptm", "md", "markdown", "html", "htm", "txt",
    "csv",
];

impl Tool {
    pub const ALL: [Tool; 18] = [
        Tool::Video,
        Tool::Mp3,
        Tool::Audio,
        Tool::Wav,
        Tool::Optimize,
        Tool::Resize,
        Tool::Webp,
        Tool::Avif,
        Tool::Jpg,
        Tool::Png,
        Tool::Crop,
        Tool::Rmbg,
        Tool::Pdf,
        Tool::Svg,
        Tool::Json,
        Tool::Links,
        Tool::Zip,
        Tool::Paste,
    ];

    /// The native alias, which is also the launcher and dispatch name.
    pub fn command(self) -> &'static str {
        match self {
            Tool::Video => "justvideo",
            Tool::Mp3 => "justmp3",
            Tool::Audio => "justaudio",
            Tool::Wav => "justwav",
            Tool::Optimize => "justoptimize",
            Tool::Resize => "justresize",
            Tool::Webp => "justwebp",
            Tool::Avif => "justavif",
            Tool::Jpg => "justjpg",
            Tool::Png => "justpng",
            Tool::Crop => "justcrop",
            Tool::Rmbg => "justrmbg",
            Tool::Pdf => "justpdf",
            Tool::Svg => "justsvg",
            Tool::Json => "justjson",
            Tool::Links => "justlinks",
            Tool::Zip => "justzip",
            Tool::Paste => "justpaste",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Video => "JustVideo",
            Tool::Mp3 => "JustMP3",
            Tool::Audio => "JustAudio",
            Tool::Wav => "JustWAV",
            Tool::Optimize => "JustOptimize",
            Tool::Resize => "JustResize",
            Tool::Webp => "JustWebP",
            Tool::Avif => "JustAVIF",
            Tool::Jpg => "JustJPG",
            Tool::Png => "JustPNG",
            Tool::Crop => "JustCrop",
            Tool::Rmbg => "JustRMBG",
            Tool::Pdf => "JustPDF",
            Tool::Svg => "JustSVG",
            Tool::Json => "JustJSON",
            Tool::Links => "JustLinks",
            Tool::Zip => "JustZip",
            Tool::Paste => "JustPaste",
        }
    }

    /// Whether the tool can read a file with this lowercase extension.
    ///
    /// A converter never lists its own target format: turning an MP3 into an
    /// MP3 needs the tool's explicit re-encode switch, not a one-click entry.
    pub fn accepts(self, extension: &str) -> bool {
        let within = |list: &[&str]| list.contains(&extension);
        match self {
            Tool::Video => within(VIDEO),
            Tool::Mp3 => (within(VIDEO) || within(AUDIO)) && extension != "mp3",
            Tool::Audio => (within(VIDEO) || within(AUDIO)) && extension != "m4a",
            Tool::Wav => (within(VIDEO) || within(AUDIO)) && extension != "wav",
            Tool::Optimize | Tool::Resize | Tool::Jpg | Tool::Rmbg => within(STILL),
            Tool::Webp => within(WEBP_SOURCES),
            Tool::Avif => within(AVIF_SOURCES),
            Tool::Png => extension == "png",
            Tool::Crop => within(ALPHA),
            Tool::Pdf => extension == "pdf",
            Tool::Svg => extension == "svg",
            Tool::Json => extension == "json",
            Tool::Links => within(LINK_SOURCES),
            // Folder tools take the folder itself, never a file in it.
            Tool::Zip | Tool::Paste => false,
        }
    }

    fn index(self) -> usize {
        Tool::ALL
            .iter()
            .position(|tool| *tool == self)
            .unwrap_or_default()
    }
}

/// The lowercase extension of a file name or path, without the dot.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

/// Every extension the menu is registered for, sorted and without dots.
///
/// `.ts` and `.mts` are deliberately absent: they are far more often
/// TypeScript than transport streams.
pub fn extensions() -> Vec<&'static str> {
    let mut all: Vec<&str> = [VIDEO, AUDIO, STILL, LINK_SOURCES, &["svg", "json"]]
        .concat()
        .to_vec();
    all.sort_unstable();
    all.dedup();
    all
}

/// What the user right-clicked, reduced to what the menu depends on.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    accepted: [u32; Tool::ALL.len()],
    pub files: u32,
    pub folders: u32,
    /// The only selected folder is a Git working tree.
    pub git: bool,
    video: u32,
}

impl Selection {
    pub fn add_file(&mut self, path: &str) {
        self.files += 1;
        let Some(extension) = extension(path) else {
            return;
        };
        if VIDEO.contains(&extension.as_str()) {
            self.video += 1;
        }
        for tool in Tool::ALL {
            if tool.accepts(&extension) {
                self.accepted[tool.index()] += 1;
            }
        }
    }

    pub fn add_folder(&mut self, git: bool) {
        self.folders += 1;
        self.git = git && self.folders == 1;
    }

    /// How many selected files the tool can read.
    pub fn count(&self, tool: Tool) -> u32 {
        self.accepted[tool.index()]
    }

    fn has(&self, tool: Tool) -> bool {
        self.count(tool) > 0
    }
}

/// Artwork for an entry. The discriminant is the icon's resource ID in the
/// shell extension, so values are stable and start at 101.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Icon {
    None = 0,
    Brand = 101,
    Video = 102,
    Audio = 103,
    Optimize = 104,
    Resize = 105,
    Convert = 106,
    Crop = 107,
    Cutout = 108,
    Pdf = 109,
    Svg = 110,
    Json = 111,
    Links = 112,
    Zip = 113,
    Console = 114,
    Options = 115,
    Paste = 116,
}

impl Icon {
    pub const ALL: [Icon; 16] = [
        Icon::Brand,
        Icon::Video,
        Icon::Audio,
        Icon::Optimize,
        Icon::Resize,
        Icon::Convert,
        Icon::Crop,
        Icon::Cutout,
        Icon::Pdf,
        Icon::Svg,
        Icon::Json,
        Icon::Links,
        Icon::Zip,
        Icon::Console,
        Icon::Options,
        Icon::Paste,
    ];

    pub fn resource_id(self) -> u16 {
        self as u16
    }

    /// The artwork's file stem under `explorer/icons`.
    pub fn file_stem(self) -> &'static str {
        match self {
            Icon::None => "",
            Icon::Brand => "brand",
            Icon::Video => "video",
            Icon::Audio => "audio",
            Icon::Optimize => "optimize",
            Icon::Resize => "resize",
            Icon::Convert => "convert",
            Icon::Crop => "crop",
            Icon::Cutout => "cutout",
            Icon::Pdf => "pdf",
            Icon::Svg => "svg",
            Icon::Json => "json",
            Icon::Links => "links",
            Icon::Zip => "zip",
            Icon::Console => "console",
            Icon::Options => "options",
            Icon::Paste => "paste",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Runs the [`Action`] with this ID.
    Action(&'static str),
    Separator,
}

/// One row of the JustTools flyout.
///
/// The menu is deliberately a single level deep: the Windows 11 context menu
/// draws the flyout of a top-level entry but leaves a flyout nested inside it
/// empty, so every choice has to be a row of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub title: String,
    pub icon: Icon,
    pub kind: Kind,
}

fn entry(title: impl Into<String>, icon: Icon, id: &'static str) -> Item {
    debug_assert!(action(id).is_some(), "unknown action {id}");
    Item {
        title: title.into(),
        icon,
        kind: Kind::Action(id),
    }
}

fn separator() -> Item {
    Item {
        title: String::new(),
        icon: Icon::None,
        kind: Kind::Separator,
    }
}

/// Drop separators that would open, close, or double up the menu.
fn tidy(items: Vec<Item>) -> Vec<Item> {
    let mut result: Vec<Item> = Vec::with_capacity(items.len());
    for item in items {
        if item.kind == Kind::Separator
            && result
                .last()
                .is_none_or(|last| last.kind == Kind::Separator)
        {
            continue;
        }
        result.push(item);
    }
    if result
        .last()
        .is_some_and(|last| last.kind == Kind::Separator)
    {
        result.pop();
    }
    result
}

/// The label Explorer shows for the whole JustTools entry.
pub const ROOT_TITLE: &str = "JustTools";

/// The entries to show for a selection, in display order.
///
/// An empty result means JustTools has nothing to offer and stays hidden.
pub fn menu(selection: &Selection) -> Vec<Item> {
    let mut items = Vec::new();
    if selection.files > 0 {
        file_items(selection, &mut items);
    } else if selection.folders > 0 {
        folder_items(selection, &mut items);
    }
    let mut items = tidy(items);
    if !items.is_empty() {
        items.push(separator());
        items.push(entry("Open JustTools here", Icon::Console, "browse"));
    }
    items
}

fn file_items(selection: &Selection, items: &mut Vec<Item>) {
    if selection.has(Tool::Video) {
        for (label, id) in [
            ("480p", "video.480"),
            ("720p (default)", "video.720"),
            ("1080p", "video.1080"),
            ("1440p", "video.1440"),
            ("4K", "video.2160"),
            ("source resolution", "video.source"),
        ] {
            items.push(entry(format!("JustVideo · {label}"), Icon::Video, id));
        }
        items.push(entry(
            "JustVideo · more options…",
            Icon::Options,
            "video.options",
        ));
        items.push(separator());
    }
    // A selection made only of videos is having its soundtrack pulled out;
    // anything else is a plain format conversion.
    let verb = if selection.video == selection.files {
        "extract"
    } else {
        "convert to"
    };
    for (tool, format, id) in [
        (Tool::Mp3, "MP3", "mp3"),
        (Tool::Audio, "M4A", "audio"),
        (Tool::Wav, "WAV", "wav"),
    ] {
        if selection.has(tool) {
            items.push(entry(
                format!("{} · {verb} {format}", tool.name()),
                Icon::Audio,
                id,
            ));
        }
    }
    items.push(separator());

    if selection.has(Tool::Optimize) {
        items.push(entry(
            "JustOptimize · smallest web format",
            Icon::Optimize,
            "optimize",
        ));
    }
    if selection.has(Tool::Resize) {
        for (label, id) in [
            ("fit 1024 px", "resize.1024"),
            ("fit 1920 px (default)", "resize.1920"),
            ("fit 2560 px", "resize.2560"),
            ("fit 3840 px", "resize.3840"),
        ] {
            items.push(entry(format!("JustResize · {label}"), Icon::Resize, id));
        }
        items.push(entry(
            "JustResize · more options…",
            Icon::Options,
            "resize.options",
        ));
    }
    items.push(separator());
    if selection.has(Tool::Webp) {
        items.push(entry("JustWebP · convert to WebP", Icon::Convert, "webp"));
    }
    if selection.has(Tool::Avif) {
        items.push(entry("JustAVIF · convert to AVIF", Icon::Convert, "avif"));
    }
    if selection.has(Tool::Jpg) {
        items.push(entry("JustJPG · convert to JPEG", Icon::Convert, "jpg"));
    }
    if selection.has(Tool::Png) {
        items.push(entry("JustPNG · compress in place", Icon::Convert, "png"));
    }
    items.push(separator());
    if selection.has(Tool::Crop) {
        items.push(entry(
            "JustCrop · trim transparent edges",
            Icon::Crop,
            "crop",
        ));
    }
    if selection.has(Tool::Rmbg) {
        items.push(entry("JustRMBG · remove background", Icon::Cutout, "rmbg"));
    }
    items.push(separator());

    let pdfs = selection.count(Tool::Pdf);
    if pdfs > 0 {
        if pdfs > 1 {
            items.push(entry(
                format!("JustPDF · merge {pdfs} PDFs"),
                Icon::Pdf,
                "pdf.merge",
            ));
        }
        for (label, id) in [
            ("split into pages", "pdf.split"),
            ("save embedded images", "pdf.images"),
            ("save links", "pdf.links"),
            ("show page count and details", "pdf.info"),
        ] {
            items.push(entry(format!("JustPDF · {label}"), Icon::Pdf, id));
        }
        items.push(entry(
            "JustPDF · more options…",
            Icon::Options,
            "pdf.options",
        ));
        items.push(separator());
    }
    if selection.has(Tool::Svg) {
        items.push(entry("JustSVG · optimize in place", Icon::Svg, "svg"));
    }
    if selection.has(Tool::Json) {
        for (label, id) in [
            ("format in place", "json.format"),
            ("minify in place", "json.minify"),
            ("validate", "json.check"),
        ] {
            items.push(entry(format!("JustJSON · {label}"), Icon::Json, id));
        }
    }
    if selection.has(Tool::Links) {
        items.push(entry(
            "JustLinks · links to links.txt",
            Icon::Links,
            "links.txt",
        ));
        items.push(entry(
            "JustLinks · links.csv with titles",
            Icon::Links,
            "links.csv",
        ));
    }
}

fn folder_items(selection: &Selection, items: &mut Vec<Item>) {
    if selection.folders == 1 {
        items.push(entry(
            "JustPaste · download the copied link here",
            Icon::Paste,
            "paste",
        ));
        items.push(separator());
        // Each opens the tool's launcher in the folder, where a blank input
        // already means "this folder".
        for (tool, icon, id) in [
            (Tool::Optimize, Icon::Optimize, "folder.optimize"),
            (Tool::Resize, Icon::Resize, "folder.resize"),
            (Tool::Webp, Icon::Convert, "folder.webp"),
            (Tool::Video, Icon::Video, "folder.video"),
            (Tool::Mp3, Icon::Audio, "folder.mp3"),
            (Tool::Links, Icon::Links, "folder.links"),
        ] {
            items.push(entry(format!("{} in this folder…", tool.name()), icon, id));
        }
        items.push(separator());
    }
    if selection.git {
        items.push(entry("JustZip · archive Git repository", Icon::Zip, "zip"));
    }
}

/// How a chosen entry starts its tool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Run {
    /// Run headless straight away with the preset arguments.
    Headless,
    /// Open the tool's console launcher with the selection filled in.
    Launcher,
    /// Open the tool's console launcher inside the selected folder.
    LauncherHere,
    /// Open the `just` tool browser inside the folder.
    Browse,
}

/// Where a headless run writes its results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Output {
    /// The tool's own beside-source default.
    Default,
    /// Pass `--output <source folder>`, which is how the tools that would
    /// otherwise consume their source are told to keep it.
    SourceFolder,
}

/// What one menu entry does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Action {
    pub id: &'static str,
    /// `None` only for [`Run::Browse`].
    pub tool: Option<Tool>,
    /// Arguments placed before the selected paths.
    pub args: &'static [&'static str],
    pub run: Run,
    pub output: Output,
    /// The tool takes exactly one input, so run it once per file.
    pub per_file: bool,
    /// The fewest accepted files the entry needs.
    pub minimum: u32,
    /// The result is something to read, so the window stays open.
    pub hold: bool,
}

const fn headless(id: &'static str, tool: Tool, args: &'static [&'static str]) -> Action {
    Action {
        id,
        tool: Some(tool),
        args,
        run: Run::Headless,
        output: Output::Default,
        per_file: false,
        minimum: 1,
        hold: false,
    }
}

const fn launcher(id: &'static str, tool: Tool) -> Action {
    Action {
        run: Run::Launcher,
        ..headless(id, tool, &[])
    }
}

const fn launcher_here(id: &'static str, tool: Tool) -> Action {
    Action {
        run: Run::LauncherHere,
        ..headless(id, tool, &[])
    }
}

const ACTIONS: &[Action] = &[
    headless("video.480", Tool::Video, &["--resolution", "480p"]),
    headless("video.720", Tool::Video, &["--resolution", "720p"]),
    headless("video.1080", Tool::Video, &["--resolution", "1080p"]),
    headless("video.1440", Tool::Video, &["--resolution", "1440p"]),
    headless("video.2160", Tool::Video, &["--resolution", "4k"]),
    headless("video.source", Tool::Video, &["--resolution", "source"]),
    launcher("video.options", Tool::Video),
    headless("mp3", Tool::Mp3, &[]),
    headless("audio", Tool::Audio, &[]),
    headless("wav", Tool::Wav, &[]),
    headless("optimize", Tool::Optimize, &[]),
    headless("resize.1024", Tool::Resize, &["--max", "1024"]),
    headless("resize.1920", Tool::Resize, &["--max", "1920"]),
    headless("resize.2560", Tool::Resize, &["--max", "2560"]),
    headless("resize.3840", Tool::Resize, &["--max", "3840"]),
    launcher("resize.options", Tool::Resize),
    Action {
        output: Output::SourceFolder,
        ..headless("webp", Tool::Webp, &[])
    },
    Action {
        output: Output::SourceFolder,
        ..headless("avif", Tool::Avif, &[])
    },
    headless("jpg", Tool::Jpg, &[]),
    headless("png", Tool::Png, &[]),
    headless("crop", Tool::Crop, &[]),
    headless("rmbg", Tool::Rmbg, &[]),
    Action {
        minimum: 2,
        ..headless("pdf.merge", Tool::Pdf, &["merge"])
    },
    Action {
        per_file: true,
        ..headless("pdf.split", Tool::Pdf, &["split"])
    },
    Action {
        per_file: true,
        ..headless("pdf.images", Tool::Pdf, &["images"])
    },
    Action {
        per_file: true,
        ..headless("pdf.links", Tool::Pdf, &["links"])
    },
    Action {
        hold: true,
        ..headless("pdf.info", Tool::Pdf, &["info"])
    },
    launcher("pdf.options", Tool::Pdf),
    headless("svg", Tool::Svg, &[]),
    headless("json.format", Tool::Json, &[]),
    headless("json.minify", Tool::Json, &["--minify"]),
    Action {
        hold: true,
        ..headless("json.check", Tool::Json, &["--check"])
    },
    headless("links.txt", Tool::Links, &[]),
    headless("links.csv", Tool::Links, &["--csv"]),
    headless("zip", Tool::Zip, &[]),
    headless("paste", Tool::Paste, &["--clipboard"]),
    launcher_here("folder.optimize", Tool::Optimize),
    launcher_here("folder.resize", Tool::Resize),
    launcher_here("folder.webp", Tool::Webp),
    launcher_here("folder.video", Tool::Video),
    launcher_here("folder.mp3", Tool::Mp3),
    launcher_here("folder.links", Tool::Links),
    Action {
        id: "browse",
        tool: None,
        args: &[],
        run: Run::Browse,
        output: Output::Default,
        per_file: false,
        minimum: 0,
        hold: false,
    },
];

/// The action behind a menu entry's ID.
pub fn action(id: &str) -> Option<Action> {
    ACTIONS.iter().copied().find(|action| action.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(files: &[&str]) -> Selection {
        let mut selection = Selection::default();
        for file in files {
            selection.add_file(file);
        }
        selection
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.title.as_str()).collect()
    }

    fn ids(items: &[Item]) -> Vec<&'static str> {
        items
            .iter()
            .filter_map(|item| match item.kind {
                Kind::Action(id) => Some(id),
                Kind::Separator => None,
            })
            .collect()
    }

    #[test]
    fn videos_offer_every_resolution_and_audio_extraction_in_one_level() {
        let items = menu(&selection(&[r"C:\clips\holiday.MOV"]));
        assert_eq!(
            titles(&items),
            [
                "JustVideo · 480p",
                "JustVideo · 720p (default)",
                "JustVideo · 1080p",
                "JustVideo · 1440p",
                "JustVideo · 4K",
                "JustVideo · source resolution",
                "JustVideo · more options…",
                "",
                "JustMP3 · extract MP3",
                "JustAudio · extract M4A",
                "JustWAV · extract WAV",
                "",
                "Open JustTools here",
            ]
        );
        assert_eq!(
            action("video.2160").unwrap().args,
            ["--resolution", "4k"].as_slice()
        );
        assert_eq!(
            action("video.1080").unwrap().args,
            ["--resolution", "1080p"].as_slice()
        );
    }

    #[test]
    fn converters_never_offer_their_own_format() {
        let items = menu(&selection(&["song.mp3"]));
        assert_eq!(
            titles(&items),
            [
                "JustAudio · convert to M4A",
                "JustWAV · convert to WAV",
                "",
                "Open JustTools here",
            ]
        );
        assert!(!titles(&menu(&selection(&["a.webp"]))).contains(&"JustWebP · convert to WebP"));
    }

    #[test]
    fn images_group_without_stray_separators() {
        let items = menu(&selection(&["photo.jpg"]));
        assert_eq!(
            titles(&items),
            [
                "JustOptimize · smallest web format",
                "JustResize · fit 1024 px",
                "JustResize · fit 1920 px (default)",
                "JustResize · fit 2560 px",
                "JustResize · fit 3840 px",
                "JustResize · more options…",
                "",
                "JustWebP · convert to WebP",
                "JustAVIF · convert to AVIF",
                "JustJPG · convert to JPEG",
                "",
                "JustRMBG · remove background",
                "",
                "Open JustTools here",
            ]
        );
        let png = menu(&selection(&["logo.png"]));
        assert!(titles(&png).contains(&"JustPNG · compress in place"));
        assert!(titles(&png).contains(&"JustCrop · trim transparent edges"));
    }

    #[test]
    fn merge_needs_two_pdfs_and_single_input_operations_run_per_file() {
        let one = menu(&selection(&["a.pdf"]));
        assert_eq!(one[0].title, "JustPDF · split into pages");
        let two = menu(&selection(&["a.pdf", "b.pdf"]));
        assert_eq!(two[0].title, "JustPDF · merge 2 PDFs");
        assert_eq!(action("pdf.merge").unwrap().minimum, 2);
        assert!(action("pdf.split").unwrap().per_file);
        assert!(action("pdf.info").unwrap().hold);
    }

    #[test]
    fn source_consuming_converters_are_told_to_keep_their_source() {
        for id in ["webp", "avif"] {
            assert_eq!(action(id).unwrap().output, Output::SourceFolder);
        }
        assert_eq!(action("jpg").unwrap().output, Output::Default);
    }

    #[test]
    fn folders_open_launchers_and_only_git_trees_offer_zip() {
        let mut plain = Selection::default();
        plain.add_folder(false);
        assert!(!titles(&menu(&plain)).contains(&"JustZip · archive Git repository"));
        assert_eq!(
            menu(&plain)[0].title,
            "JustPaste · download the copied link here"
        );
        assert_eq!(action("paste").unwrap().args, ["--clipboard"].as_slice());
        assert_eq!(menu(&plain)[2].title, "JustOptimize in this folder…");
        assert_eq!(action("folder.video").unwrap().run, Run::LauncherHere);

        let mut repository = Selection::default();
        repository.add_folder(true);
        assert!(titles(&menu(&repository)).contains(&"JustZip · archive Git repository"));

        let mut several = Selection::default();
        several.add_folder(true);
        several.add_folder(true);
        assert!(menu(&several).is_empty());
    }

    #[test]
    fn unsupported_selections_hide_the_menu() {
        assert!(menu(&selection(&["notes.xyz", "Makefile"])).is_empty());
        assert!(menu(&Selection::default()).is_empty());
        assert_eq!(extension(r"C:\a.b\file"), None);
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("Clip.MP4").as_deref(), Some("mp4"));
    }

    #[test]
    fn every_entry_resolves_and_every_action_is_reachable() {
        let mut reachable = std::collections::HashSet::new();
        for extension in extensions() {
            let one = format!("file.{extension}");
            let two = format!("other.{extension}");
            let items = menu(&selection(&[&one, &two]));
            assert!(!items.is_empty(), ".{extension} is registered for nothing");
            for id in ids(&items) {
                let action = action(id).unwrap_or_else(|| panic!("no action {id}"));
                assert_eq!(action.id, id);
                reachable.insert(id);
            }
        }
        let mut repository = Selection::default();
        repository.add_folder(true);
        reachable.extend(ids(&menu(&repository)));

        assert!(!extensions().contains(&"ts"));
        let mut seen = std::collections::HashSet::new();
        for action in ACTIONS {
            assert!(seen.insert(action.id), "duplicate action {}", action.id);
            assert!(
                reachable.contains(action.id),
                "no menu entry runs {}",
                action.id
            );
        }
        for icon in Icon::ALL {
            assert!(!icon.file_stem().is_empty());
        }
        assert_eq!(format!("{CLASS_ID_VALUE:032X}"), CLASS_ID.replace('-', ""));
    }
}
