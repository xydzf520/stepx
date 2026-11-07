//! A chooser waits on its own worker, leaving UI rendering and phone sync free.
use super::{Companion, ViewContext};
use std::{path::PathBuf, sync::mpsc};
use stepx_companion::file_dialog::{DialogKind, FileDialog};

type Completion = Box<
    dyn FnOnce(&mut Companion, Vec<PathBuf>, &mut ViewContext<Companion>) -> anyhow::Result<()>,
>;

#[derive(Default)]
pub(super) struct FileDialogState {
    pending: Option<(mpsc::Receiver<Result<Vec<PathBuf>, String>>, Completion)>,
}

impl Companion {
    pub(super) fn ensure_file_dialog_device(
        &self,
        expected: &Option<String>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            &self.selected.as_ref().map(|d| d.serial.clone()) == expected,
            "所选手机已变化，请重新选择文件；原文件和草稿未更改"
        );
        Ok(())
    }
    pub(super) fn choose_paths(
        &mut self,
        dialog: FileDialog,
        kind: DialogKind,
        complete: impl FnOnce(&mut Self, Vec<PathBuf>, &mut ViewContext<Self>) -> anyhow::Result<()>
            + 'static,
    ) {
        if self.file_dialog.pending.is_some() {
            self.status = "文件选择窗口已打开，请先选择文件或取消".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.file_dialog.pending = Some((rx, Box::new(complete)));
        if let Err(error) = std::thread::Builder::new()
            .name("file-chooser".into())
            .spawn(move || {
                let result = dialog.show(kind).map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
            })
        {
            self.file_dialog.pending = None;
            self.status = format!("无法打开文件选择窗口：{error}；请重试");
            self.error = true;
        }
    }

    pub(super) fn choose_path(
        &mut self,
        dialog: FileDialog,
        kind: DialogKind,
        complete: impl FnOnce(&mut Self, PathBuf, &mut ViewContext<Self>) -> anyhow::Result<()>
            + 'static,
    ) {
        self.choose_paths(dialog, kind, move |app, paths, ctx| {
            if let Some(path) = paths.into_iter().next() {
                complete(app, path, ctx)
            } else {
                Ok(())
            }
        });
    }

    pub(super) fn poll_file_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        let Some((rx, _)) = &self.file_dialog.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("文件选择器意外关闭，请重试".into()),
        };
        let (_, complete) = self.file_dialog.pending.take().unwrap();
        let result = match result {
            Ok(paths) if paths.is_empty() => Ok(()), // Cancel leaves drafts untouched.
            Ok(paths) => complete(self, paths, ctx),
            Err(error) => Err(anyhow::anyhow!(error)),
        };
        if let Err(error) = result {
            self.error = true;
            self.status = format!("{error:#}");
        }
        ctx.notify();
    }
}
