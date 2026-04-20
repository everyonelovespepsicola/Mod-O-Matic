# Mod-O-Matic 🚀

Download Here https://www.nexusmods.com/site/mods/1825

1.png 2.png
3.png 4.png

**Mod-O-Matic** is a high-performance utility suite for power users, featuring four powerful tools in one lightning-fast application:
*   **Replace-O-Matic:** A bulk file replacer with deep binary safety checks, designed for game modders.
*   **Sort-O-Matic:** A flexible file sorter for organizing massive collections of assets.
*   **Merge-O-MaticINI:** A smart compiler to inject multiple tweak parameters directly into main INI configuration files.
*   **Authent-O-Matic:** A deep binary analyzer to track down misnamed files and corrupted headers.

---

## ✨ Awesome Features

*   **Quad-App Interface:** Instantly switch between the 4 core tools with a clean tab interface.
* **Blazing Fast at Any Scale:** Built from the ground up to handle massive workloads. Drop a folder with **50,000+ files** and watch it load instantly. Thanks to UI virtualization and multi-threaded processing, the app remains smooth and responsive no matter the size of your project.
* **Sleek Drag-and-Drop UI:** Built on a custom-themed, ultra-responsive graphical interface. Drop individual files or **entire folders**! The intelligent routing system automatically sifts through folder trees to find all valid files, puts the master file in the top box, and queues everything else in the bottom list.
* **Live Animated Progress Bar:** Replacing a massive batch of files? Watch the app work in real-time at 60 FPS with a smooth progress bar and a live file counter.
* **Dynamic UI States:** The interface holds your hand every step of the way. Buttons glow neon green when ready, gray out when unavailable, and seamlessly guide you through the correct workflow.

---

## 🗃️ Sort-O-Matic

`Sort-O-Matic` is designed to bring order to chaos. Drop entire folders containing thousands of mixed-format files and quickly sort them into a clean, organized structure.

### Features
*   **Custom Extension Filtering:** Specify exactly which file types you want to sort by entering a comma-separated list of extensions (e.g., `png, jpg, dds, txt`).
*   **Flexible Sorting Modes:**
    *   **Group by Extension (Default):** Creates a sub-folder for each extension and groups the files accordingly (e.g., `/png`, `/jpg`).
    *   **Preserve Structure:** Recreates the exact original folder structure of your source files at the destination.
    *   **Single Folder:** Copies all matching files directly into the destination folder with no sub-folders.
*   **Safe Output Folder:** By default, all sorted files are placed into a new "Sorted_Files" folder to keep your destination directory clean. This can be disabled.

### How to Use (Sort-O-Matic)
1.  Select the **Sort-O-Matic** tab.
2.  Enter the file extensions you want to sort.
3.  **Drag & Drop** your files or folders into the main window.
4.  Choose your desired **Sort Behavior**.
5.  Select a **Destination** folder.
6.  Click the glowing green **`[Sort Files]`** button!

---

## 🕵️ Authent-O-Matic

`Authent-O-Matic` is a diagnostic tool for modders. Have a mysterious crash caused by a misnamed file? This tool will find it.

### Features
*   **Magic Number Sniffing:** Drops a folder and it will instantly open and read the internal binary headers (magic numbers) of every single file.
*   **Mismatch Detection:** Automatically compares the file's extension to its actual internal format. If a `.png` file is actually a renamed `.dds` file (or is 0-bytes empty), it flags it!

### How to Use (Authent-O-Matic)
1. Select the **Authent-O-Matic** tab.
2. **Drag & Drop** any folder into the window. It will scan in the background and instantly list the culprits!

## ⚙️ Merge-O-MaticINI

`Merge-O-MaticINI` allows you to take one primary INI configuration file and layer multiple "tweak" INI files over it. Perfect for combining multiple game mods into a single configuration file!

### Features
*   **Intelligent Overwriting:** Tweak files at the bottom of the list overwrite tweaks from files above them, allowing you to perfectly resolve conflicts.
*   **Safe Saving:** You can either create a new `_Merged.ini` file or overwrite the original while generating an instant `.bak` backup.

### How to Use (Merge-O-MaticINI)
1. Select the **Merge-O-MaticINI** tab.
2. **Drag & Drop** your main `.ini` configuration file into the top panel.
3. **Drag & Drop** your tweak `.ini` files into the central panel. Drag order matters!
4. Click the glowing green **`[Merge Files]`** button!

---

## ↔️ Replace-O-Matic

### Features
* **Intelligent Export Modes:** Choose between two powerful output options from a simple dropdown:
    *   **Safe Export (Default):** This is the recommended mode for modding. It creates a clean `_Replacements` folder at your chosen destination, then copies and renames all the necessary files, **perfectly recreating the original folder structure**. Your original files are never touched.
    *   **Overwrite:** For power users who need to modify files in-place, this mode directly overwrites the target files.
* **Bulletproof Safety Nets:** Accidentally replacing a `.dds` texture file with a `.png` file can completely crash a game engine. Replace-O-Matic includes a **Universal Safety Scanner** to prevent catastrophic mistakes:
    * **Strict Extension Matching:** If your Master file is a `.dds`, the app will automatically reject any `.txt`, `.png`, or `.mp3` files you accidentally drop into the target list.
    * **Deep "Magic Number" Sniffing:** What if you accidentally renamed a `.png` to `.dds`? Replace-O-Matic physically opens both files, reads their first 4 binary bytes, and compares their internal digital fingerprints. If a file is lying about its format, Replace-O-Matic catches the mismatch and immediately rejects the file from the processing queue!
    * **Dynamic Auto-Purge:** If you have 50 files queued up and you change your mind and swap the Master file to a different format, the app will instantly rescan your entire queue to protect you, automatically purging any incompatible files.

### Advanced Modding: Running Without Safety
Sometimes you *know* exactly what you are doing. If you are intentionally replacing a file with a weird custom extension, or you are forcing a game engine to accept a new file format, the safety scanner might block your workflow.

To bypass the safety nets:
1. Look for the **"DANGER: Disable Format Safety Checks"** checkbox in the bottom right corner of the app.
2. Click the box (it will glow red to remind you that you are in the danger zone).
3. **All extension and binary signature checks are now disabled.** You can force Replace-O-Matic to replace *any* file with *any* other file. 

*Note: If you disable the safety checks, load mismatched files, and then turn the safety checks back on, the app will instantly scan your list and automatically purge the dangerous files!*

### How to Use (Replace-O-Matic)
1. Select the **Replace-O-Matic** tab.
2. **Set the Master:** Click `[Find...]` or **Drag & Drop** your new replacement file into the top box. 
3. **Queue the Targets:** Click `[Find...]` or **Drag & Drop** all the old files you want to overwrite into the bottom list.
4. **Select Output Mode:** Use the dropdown to choose between `Safe Export` (default, creates a new folder) or `Overwrite` (modifies original files).
5. **Replace:** Click the glowing green **`[Replace]`** button!

---

## 📈 Performance at Scale

There is **no hardcoded limit** to the number of files you can process at once! 

Replace-O-Matic was stress-tested with over **50,000 files** in a single batch. Thanks to its multi-threaded Rust architecture, efficient data handling, and UI virtualization, it remains incredibly memory-efficient and responsive. It scales directly with your computer's hardware, allowing you to queue up massive projects without freezing or crashing.

---

### Requirements
* Windows 10 / 11
* No installation required. Just run `Mod-O-Matic.exe`!
