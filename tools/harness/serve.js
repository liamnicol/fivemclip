// A static server for ui/ that honours Range requests.
//
// Not a detail. `python -m http.server` answers a Range with the whole file and
// a 200, and Chromium reads that as "this stream is not seekable": <video>
// reports seekable=[0,0], every seek silently does nothing, and the trimmer
// looks broken when it is not. Half a day went into that once.
const fs = require("fs");
const http = require("http");
const path = require("path");

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".mp4": "video/mp4",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".svg": "image/svg+xml",
  ".json": "application/json",
};

/** Serve `roots` (a map of URL prefix to directory) on an ephemeral port. */
function serve(roots) {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    const prefix = Object.keys(roots)
      .sort((a, b) => b.length - a.length)
      .find((p) => url.pathname === p || url.pathname.startsWith(p.endsWith("/") ? p : `${p}/`));
    if (!prefix) return res.writeHead(404).end("no root");

    const rest = decodeURIComponent(url.pathname.slice(prefix.length)).replace(/^\/+/, "");
    // Contained deliberately: a harness that will serve ../../etc/passwd is a
    // harness someone will eventually point at something that matters.
    const root = path.resolve(roots[prefix]);
    const file = path.resolve(root, rest || "index.html");
    if (file !== root && !file.startsWith(root + path.sep)) {
      return res.writeHead(403).end("outside the root");
    }

    let stat;
    try {
      stat = fs.statSync(file);
    } catch {
      return res.writeHead(404).end("not found");
    }

    const type = TYPES[path.extname(file).toLowerCase()] ?? "application/octet-stream";
    const range = req.headers.range;
    if (range) {
      const m = /^bytes=(\d*)-(\d*)$/.exec(range.trim());
      if (m) {
        // An open-ended suffix range ("bytes=-500") means the last N bytes, not
        // from 0 to N. Getting that backwards serves the head of the file for a
        // seek to the end, which looks like a corrupt video rather than a bug.
        let start = m[1] === "" ? stat.size - Number(m[2]) : Number(m[1]);
        let end = m[1] === "" || m[2] === "" ? stat.size - 1 : Number(m[2]);
        start = Math.max(0, start);
        end = Math.min(stat.size - 1, end);
        if (start > end) {
          return res
            .writeHead(416, { "Content-Range": `bytes */${stat.size}` })
            .end();
        }
        res.writeHead(206, {
          "Content-Type": type,
          "Content-Length": end - start + 1,
          "Content-Range": `bytes ${start}-${end}/${stat.size}`,
          "Accept-Ranges": "bytes",
        });
        return fs.createReadStream(file, { start, end }).pipe(res);
      }
    }

    res.writeHead(200, {
      "Content-Type": type,
      "Content-Length": stat.size,
      "Accept-Ranges": "bytes",
    });
    fs.createReadStream(file).pipe(res);
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () =>
      resolve({ server, port: server.address().port })
    );
  });
}

module.exports = { serve };
