// Runs only in an independent /usr/bin/osascript process. No StepX AppKit
// objects, nested client runloops, System Events, shell commands or UI scripting.
function run(argv) {
    try {
        var request = JSON.parse(argv[0]);
        var app = Application.currentApplication();
        app.includeStandardAdditions = true;
        var options = {withPrompt: request.title || "选择文件"};
        if (request.location) options.defaultLocation = Path(request.location);
        var selected;
        if (request.kind === "save_file") {
            if (request.filename) options.defaultName = request.filename;
            selected = app.chooseFileName(options);
        } else if (request.kind === "open_folder") {
            selected = app.chooseFolder(options);
        } else if (request.kind === "open_file" || request.kind === "open_files") {
            options.multipleSelectionsAllowed = request.kind === "open_files";
            if (request.extensions && request.extensions.length) options.ofType = request.extensions;
            selected = app.chooseFile(options);
        } else {
            throw new Error("Unsupported file selector mode");
        }
        var paths = Array.isArray(selected) ? selected : [selected];
        return JSON.stringify({status: "selected", paths: paths.map(function (path) { return path.toString(); })});
    } catch (error) {
        if (Number(error.errorNumber) === -128 || Number(error.number) === -128) {
            return JSON.stringify({status: "cancelled"});
        }
        return JSON.stringify({status: "error", message: String(error.message || error)});
    }
}
