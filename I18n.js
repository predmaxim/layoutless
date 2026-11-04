.pragma library
// Interface text in other languages, keyed by the English text itself, so a
// string missing from a table shows in English. tr("Don't correct in %1", x)
// fills in %1, %2… after the lookup. Quickshell plugins get no .qm
// catalogues, so Qt's qsTr isn't used. test.js checks both directions: every
// tr() string has a line here, and every line here is used.
var TABLES = {
  ru: {
    "Layoutless: daemon not running": "Layoutless: демон не запущен",
    "English": "английская",
    "Russian": "русская",
    "Layout: %1, auto-correction on": "Раскладка: %1, автоисправление включено",
    "Layout: %1, auto-correction paused": "Раскладка: %1, автоисправление на паузе",
    "Click to switch the layout, right click for settings": "Клик — сменить раскладку, правый клик — настройки",
    "The layoutless daemon is not running.\nStart it with: systemctl --user start layoutless\n(installed with --system: sudo systemctl start layoutless@$USER)":
      "Демон layoutless не запущен.\nЗапуск: systemctl --user start layoutless\n(установлен с --system: sudo systemctl start layoutless@$USER)",
    "Auto-correction": "Автоисправление",
    "Retype words typed in the wrong layout": "Перепечатывать слова, набранные не в той раскладке",
    "Layout per window": "Раскладка по окнам",
    "A window gets its layout back on focus, new ones start on English": "Окно в фокусе получает свою раскладку, новые — английскую",
    "Learned words": "Выученные слова",
    "Retyped in the other layout": "Перепечатываются в другой раскладке",
    "Ignored apps": "Программы без исправления",
    "Words aren't corrected there": "Слова в них не исправляются",
    "Don't correct in %1": "Не исправлять в %1",
    "%1 still works there": "%1 там по-прежнему работает",
    "Find or add a word: next, привет": "Найти или добавить слово: next, привет",
    "Add": "Добавить",
    "No learned words yet. Flip the same word with %1 and it's remembered.":
      "Выученных слов пока нет. Переверните слово клавишами %1 — и оно запомнится.",
    "No learned words yet. Flip the same word with %1 %2 times and it's remembered.":
      "Выученных слов пока нет. Переверните одно и то же слово клавишами %1 — после %2-го раза оно запомнится.",
    "Nothing matches. Press Enter to add it.": "Ничего не найдено. Enter — добавить.",
    "󰓡 keep the other spelling instead   󰆴 forget": "󰓡 оставить другое написание   󰆴 забыть",
    "No ignored apps yet.": "Программ без исправления пока нет."
  }
}

// Text follows LC_MESSAGES, overridden by LC_ALL and defaulting to LANG, as
// the system splits it. env is name -> value (Quickshell.env in QML).
function textLanguage(env) {
  var names = ["LC_ALL", "LC_MESSAGES", "LANG"]
  for (var i = 0; i < names.length; i++) {
    var v = String(env(names[i]) || "").split(".")[0].split("@")[0]
    if (v && v !== "C" && v !== "POSIX") {
      var l = v.slice(0, 2).toLowerCase()
      return TABLES[l] ? l : "en"
    }
  }
  return "en"
}

function translator(lang) {
  var table = TABLES[lang] || {}
  return function(text) {
    var out = Object.prototype.hasOwnProperty.call(table, text) ? table[text] : text
    for (var i = 1; i < arguments.length; i++) out = out.split("%" + i).join(String(arguments[i]))
    return out
  }
}
