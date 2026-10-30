// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package service

import (
	"bytes"
	"encoding/xml"
	"fmt"
)

func LaunchdPlist(binPath, logPath string) string {
	escape := func(s string) string { var b bytes.Buffer; _ = xml.EscapeText(&b, []byte(s)); return b.String() }
	return fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>dev.iii.mesh</string>
<key>ProgramArguments</key><array><string>%s</string><string>node</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>ExitTimeOut</key><integer>30</integer>
<key>Umask</key><integer>63</integer>
<key>EnvironmentVariables</key><dict><key>TS_NO_LOGS_NO_SUPPORT</key><string>true</string></dict>
<key>StandardOutPath</key><string>%s</string>
<key>StandardErrorPath</key><string>%s</string>
</dict></plist>
`, escape(binPath), escape(logPath), escape(logPath))
}
