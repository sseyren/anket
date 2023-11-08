.PHONY: all anket frontend clean

anket: frontend
	cargo build --release --bin anket

frontend:
	trunk build --release

clean:
	trunk clean
	cargo clean
