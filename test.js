// node test.js — every tr() string in the widget has a Russian line in
// I18n.js, and every Russian line is used.
const assert = require("assert")
const fs = require("fs")

const I = new Function(fs.readFileSync(__dirname + "/I18n.js", "utf8").replace(".pragma library", "") + "; return { TABLES, textLanguage, translator }")()
const qml = fs.readFileSync(__dirname + "/BarWidget.qml", "utf8")
const used = new Set([...qml.matchAll(/\btr\("((?:[^"\\]|\\.)*)"/g)].map(m => JSON.parse(`"${m[1]}"`)))
for (const s of used) assert.ok(Object.prototype.hasOwnProperty.call(I.TABLES.ru, s), "no ru for: " + s)
for (const s of Object.keys(I.TABLES.ru)) assert.ok(used.has(s), "unused ru line: " + s)

// The language follows LC_ALL > LC_MESSAGES > LANG; unknown ones fall back to English.
const env = vars => name => vars[name]
assert.strictEqual(I.textLanguage(env({ LANG: "en_US.UTF-8", LC_TIME: "ru_RU.UTF-8" })), "en")
assert.strictEqual(I.textLanguage(env({ LANG: "en_US.UTF-8", LC_MESSAGES: "ru_RU.UTF-8" })), "ru")
assert.strictEqual(I.textLanguage(env({ LANG: "ru_RU.UTF-8", LC_ALL: "C" })), "ru")
assert.strictEqual(I.textLanguage(env({ LANG: "de_DE.UTF-8" })), "en")
assert.strictEqual(I.translator("ru")("Don't correct in %1", "orca"), "Не исправлять в orca")

console.log("ok")
