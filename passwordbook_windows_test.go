package main

import (
	"bytes"
	"encoding/json"
	"os"
	"slices"
	"testing"
)

func TestPasswordBookRoundTrip(t *testing.T) {
	t.Setenv("LOCALAPPDATA", t.TempDir())
	book, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	const first = "模拟密码 · 第一🔐"
	const second = " 模拟密码 · 第二 "
	for _, password := range []string{first, second, first} {
		if err := book.Remember(password); err != nil {
			t.Fatal(err)
		}
	}
	stored, err := os.ReadFile(book.path)
	if err != nil {
		t.Fatal(err)
	}
	if json.Valid(stored) || bytes.Contains(stored, []byte(first)) || bytes.Contains(stored, []byte(second)) {
		t.Fatal("password book saved readable records instead of encrypted data")
	}
	reloaded, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(reloaded.Passwords(), []string{second, first}) {
		t.Fatal("password book did not preserve the original passwords and order")
	}
}

func TestPasswordBookConcurrentSaves(t *testing.T) {
	t.Setenv("LOCALAPPDATA", t.TempDir())
	first, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	second, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	const firstPassword = "模拟密码 · 并发一"
	const secondPassword = "模拟密码 · 并发二"
	results := make(chan error, 2)
	go func() { results <- first.Remember(firstPassword) }()
	go func() { results <- second.Remember(secondPassword) }()
	for range 2 {
		if err := <-results; err != nil {
			t.Error(err)
		}
	}
	reloaded, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	passwords := reloaded.Passwords()
	if len(passwords) != 2 || !slices.Contains(passwords, firstPassword) || !slices.Contains(passwords, secondPassword) {
		t.Fatal("a concurrent password book save lost an entry")
	}
}
