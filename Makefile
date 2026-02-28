.PHONY: dev build test lint fmt clean install

dev:
	pnpm tauri dev

build:
	pnpm tauri build

test:
	cd src-tauri && cargo test --all-features

lint:
	cd src-tauri && cargo clippy --all-features -- -D warnings
	pnpm lint

fmt:
	cd src-tauri && cargo fmt
	pnpm lint --fix

check:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-features -- -D warnings
	pnpm lint

clean:
	cd src-tauri && cargo clean
	rm -rf dist node_modules

install:
	pnpm install

setup: install
	@echo "Ready. Run 'make dev' to start development."
