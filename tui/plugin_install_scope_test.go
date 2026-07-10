package main

import "testing"

func TestParsePluginInstallArgs(t *testing.T) {
	cases := []struct {
		args      []string
		wantPath  string
		wantScope string
		wantErr   bool
	}{
		{[]string{"/tmp/p"}, "/tmp/p", "global", false},
		{[]string{"/tmp/p", "workspace"}, "/tmp/p", "workspace", false},
		{[]string{"--workspace", "owner/repo"}, "owner/repo", "workspace", false},
		{[]string{"https://github.com/a/b", "--global"}, "https://github.com/a/b", "global", false},
		{[]string{"-w", "./plugin"}, "./plugin", "workspace", false},
		{[]string{}, "", "", true},
		{[]string{"a", "b"}, "", "", true},
		{[]string{"workspace"}, "", "", true}, // scope alone is not a source
	}
	for _, tc := range cases {
		path, scope, err := parsePluginInstallArgs(tc.args)
		if tc.wantErr {
			if err == nil {
				t.Fatalf("args %v: expected error", tc.args)
			}
			continue
		}
		if err != nil {
			t.Fatalf("args %v: unexpected error: %v", tc.args, err)
		}
		if path != tc.wantPath || scope != tc.wantScope {
			t.Fatalf("args %v: got path=%q scope=%q, want path=%q scope=%q",
				tc.args, path, scope, tc.wantPath, tc.wantScope)
		}
	}
}
