#!/usr/bin/env python3
import tkinter as tk
import os
import subprocess

root = tk.Tk()
root.title("Clark Computer Use QA")
root.geometry("560x360")

button = tk.Button(
    root,
    text="Click to verify Clark Computer Use",
    font=("Sans", 18),
)


def clicked() -> None:
    button.configure(text="Clicked by Clark Computer Use", background="#90ee90")


button.configure(command=clicked)
button.pack(expand=True, fill=tk.BOTH)
root.update_idletasks()
for window_id in subprocess.check_output(
    ["xdotool", "search", "--name", "^Clark Computer Use QA$"],
    text=True,
).splitlines():
    subprocess.run(
        [
            "xprop",
            "-id",
            window_id,
            "-f",
            "_NET_WM_PID",
            "32c",
            "-set",
            "_NET_WM_PID",
            str(os.getpid()),
        ],
        check=True,
    )
root.mainloop()
