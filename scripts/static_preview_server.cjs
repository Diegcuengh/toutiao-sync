const http = require("http");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..", "dist");
const port = 14321;

const mime = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
};

http
  .createServer((req, res) => {
    const reqPath = req.url === "/" ? "/index.html" : String(req.url || "/").split("?")[0];
    const filePath = path.join(root, reqPath);
    fs.readFile(filePath, (error, data) => {
      if (error) {
        res.statusCode = 404;
        res.end("not found");
        return;
      }
      res.setHeader("Content-Type", mime[path.extname(filePath)] || "application/octet-stream");
      res.end(data);
    });
  })
  .listen(port, "127.0.0.1", () => {
    console.log(`preview-ready:${port}`);
  });
