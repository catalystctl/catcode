//go:build windows

package main

import "golang.org/x/sys/windows"

func sessionLockProcessAlive(pid int) bool {
	const stillActive = 259
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if err != nil {
		return false
	}
	defer windows.CloseHandle(handle)
	var code uint32
	return windows.GetExitCodeProcess(handle, &code) == nil && code == stillActive
}
