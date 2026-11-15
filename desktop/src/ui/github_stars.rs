//! Public repository metadata: one bounded request per process, never a refresh loop.
use super::*;
use facloud_ui::color::ColorU;
use std::sync::OnceLock;
const REPOSITORY: &str = "https://github.com/xydzf520/stepx";
static STARS: OnceLock<Result<u64, String>> = OnceLock::new();
#[derive(Default)]
pub(super) struct GithubStars {
    pub count: Option<u64>,
    pub pending: bool,
}
fn parse(bytes: &[u8]) -> Result<u64> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        value["full_name"].as_str() == Some("xydzf520/stepx"),
        "项目不匹配"
    );
    value["stargazers_count"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("缺少 Star 数量"))
}
fn fetch() -> Result<u64> {
    #[cfg(feature = "ui-capture")]
    if std::env::args().any(|a| a == "--verify-shell-details") {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "GITHUB_STARS_FETCH_ONCE");
    }
    // Prefer lightweight system curl before spawning Python at startup (and avoid
    // triggering macOS Xcode CLT prompts when launched from Finder).
    let mut curl = std::process::Command::new("curl");
    curl.args([
        "-fsSL",
        "--max-time",
        "5",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: StepX-Companion",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        "https://api.github.com/repos/xydzf520/stepx",
    ]);
    stepx_companion::resources::quiet_command(&mut curl);
    if let Ok(out) = stepx_companion::adb::run_process(curl, None, Duration::from_secs(6)) {
        if out.status.success() && out.stdout.len() <= 65536 {
            if let Ok(count) = parse(&out.stdout) {
                return Ok(count);
            }
        }
    }
    let mut command = stepx_companion::resources::python_command();
    command.args(["-c", r#"
import urllib.request
r=urllib.request.Request('https://api.github.com/repos/xydzf520/stepx', headers={'Accept':'application/vnd.github+json','User-Agent':'StepX-Companion','X-GitHub-Api-Version':'2022-11-28'})
with urllib.request.urlopen(r, timeout=6) as response:
    data=response.read(65537)
    if len(data)>65536: raise ValueError('response limit')
    import sys
    sys.stdout.buffer.write(data)
"#]);
    let out = stepx_companion::adb::run_process(command, None, Duration::from_secs(8))?;
    anyhow::ensure!(out.status.success(), "GitHub 暂时不可用");
    parse(&out.stdout)
}
impl Companion {
    pub(super) fn initialize_stars(&mut self, ctx: &mut ViewContext<Self>) {
        if (self.demo_mode || self.capture.is_some())
            && !std::env::args().any(|a| a == "--verify-shell-details")
        {
            return;
        }
        self.github_stars.pending = true;
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = STARS.get_or_init(|| fetch().map_err(|_| "暂时无法读取 Star 数量".into()));
            let _ = tx.send_blocking(result.clone());
        });
        ctx.spawn(async move { rx.recv().await }, |me, result, ctx| {
            me.github_stars.pending = false;
            me.github_stars.count = result.ok().and_then(Result::ok);
            ctx.notify();
        });
    }
    pub(super) fn stars_link(&self) -> Box<dyn Element> {
        let p = self.p();
        let label = match self.github_stars.count {
            Some(n) => format!("Star {n}"),
            None if self.github_stars.pending => "Star …".into(),
            None => "GitHub".into(),
        };
        Hoverable::new(self.mouse_state("github-stars"), move |mouse| {
            let fg = if mouse.is_mouse_over_element() {
                p.text
            } else {
                p.text3
            };
            Container::new(self.hstack(
                theme::S1,
                vec![
                        self.ui_icon(icons::UiIcon::GithubStar, 13., fg),
                        Text::new(label.clone(), self.mono_font, theme::CAPTION)
                            .with_color(fg)
                            .with_line_height_ratio(1.)
                            .soft_wrap(false)
                            .with_saved_char_position(0, "github-star-count".into())
                            .finish(),
                    ],
            ))
            .with_horizontal_padding(6.)
            .with_background_color(if mouse.is_mouse_over_element() {
                p.hover
            } else {
                ColorU::transparent_black()
            })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
        })
        .with_cursor(platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::StudioOpenUrl(REPOSITORY.into())))
        .finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn github_count_is_real_and_failures_are_not_zero() {
        assert_eq!(
            parse(br#"{"full_name":"xydzf520/stepx","stargazers_count":1234}"#).unwrap(),
            1234
        );
        for data in [
            br#"{"message":"API rate limit exceeded"}"#.as_slice(),
            br#"{"full_name":"other/repo","stargazers_count":999}"#,
        ] {
            assert!(parse(data).is_err());
        }
        assert!(parse(br#"{"full_name":"xydzf520/stepx","stargazers_count":-1}"#).is_err());
    }
    #[test]
    fn once_cell_keeps_failed_fetch_without_retry() {
        let once = OnceLock::new();
        let calls = std::cell::Cell::new(0);
        for _ in 0..10 {
            let _ = once.get_or_init(|| {
                calls.set(calls.get() + 1);
                Err::<u64, _>("offline")
            });
        }
        assert_eq!(calls.get(), 1);
    }
}
