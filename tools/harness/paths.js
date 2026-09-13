const path = require("path");

const UI = path.resolve(__dirname, "../../ui");

/** Where the harness keeps the media a scenario serves through
 *  convertFileSrc. Generated on demand, never committed. */
const MEDIA = process.env.FIVEMCLIP_HARNESS_MEDIA
  ? path.resolve(process.env.FIVEMCLIP_HARNESS_MEDIA)
  : path.resolve(__dirname, "media");

module.exports = { UI, MEDIA };
