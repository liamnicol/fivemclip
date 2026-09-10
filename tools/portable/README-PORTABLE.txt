FiveMClip - portable
====================

No installation. Unzip somewhere you have write access and run FiveMClip.exe.

What "portable" means here
--------------------------
Settings live in this folder, not in your Windows profile, and recordings
default to a Recordings folder beside the program. Delete the folder and
nothing is left behind. You can point recordings somewhere roomier on the
setup screen when you first run it - a USB stick is a poor home for video.

The file portable.txt is what switches this on. Delete it and FiveMClip
behaves like an installed copy.

Requirements
------------
Windows 10 or 11, and the Microsoft Edge WebView2 runtime. WebView2 ships with
Windows 11 and with most Windows 10 installs. If FiveMClip opens a blank window
or refuses to start, install the Evergreen Runtime from Microsoft:

  https://developer.microsoft.com/microsoft-edge/webview2/

The installer version sets this up for you; a portable build cannot.

First run
---------
Windows SmartScreen will warn about an unknown publisher, because this build is
not code-signed. Click More info, then Run anyway. The source is public if you
would rather check before trusting it:

  https://github.com/liamnicol/fivemclip

Default hotkeys
---------------
  F9    Save the last few minutes
  F10   Screenshot
  F11   Screenshot a region you drag
  F8    Start / stop recording a whole session

All of them are changeable in Settings.

Licence
-------
FiveMClip is MIT licensed - see LICENSE. It bundles an unmodified ffmpeg binary
which is GPL licensed and carries its own conditions; see THIRD-PARTY.md before
redistributing this folder.
