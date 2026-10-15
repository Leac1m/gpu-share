# Trying GPU Share with friends (pilot guide)

This is the practical guide for the first build (roadmap D10): one host (the owner's Windows
laptop with the RTX A2000) and a few friends' computers on the same home Wi-Fi.

## What you need

* The owner's laptop: Windows 10 or 11, NVIDIA RTX A2000 (any recent NVIDIA card works), the
  normal NVIDIA driver, and about 10 GB free disk space for the engine and two models.
* Each friend: Ubuntu 22.04 or 24.04 (or Windows), on the **same Wi-Fi** as the owner.
* The installers from a GitHub release, or from the artifacts of a Release workflow run (see
  [releasing.md](releasing.md)).

## The owner (Windows)

1. **Run the installer** (`GPU Share_<version>_x64-setup.exe`).
   * Windows will say **"Windows protected your PC"** because the installer isn't signed yet.
     Click **More info**, then **Run anyway**.
   * Near the end it asks once for administrator permission (a UAC prompt). That adds a firewall
     rule so friends on your home network can reach the app. It applies to **private** networks
     only.
2. **Open GPU Share.** The first time, it downloads the AI engine for your graphics card (a
   one-time download of a few hundred MB). The Share screen shows the progress.
3. **Press "Set up sharing"**, then open **Models** and **Add** a model:
   * **Qwen3 8B** (4.7 GB): the better one, fits an 8 GB card.
   * **Qwen3 4B** (2.3 GB): faster, lighter.
   Each download is checked against its published checksum before it is used.
4. Check the **Limits** tab. Every rule is on by default. The one people notice first is
   **"Pause while I'm using the computer"**: sharing waits until you've been away from the
   keyboard and mouse for 5 minutes. Turn it off, or shorten the wait, to try things quickly.
5. Turn on **Share my graphics card**.
6. If the Share screen says **"Friends can't reach this computer"**, Windows has your Wi-Fi set to
   *Public*. Open **Settings → Network & internet**, pick your Wi-Fi, and set **Network profile
   type** to **Private**.

The app lives in the tray next to the clock. Closing the window keeps it running; use the tray
icon to switch sharing on or off, or to quit.

## Each friend (Ubuntu)

1. Install the package: `sudo apt install "./GPU Share_<version>_amd64.deb"` (the quotes matter: the
   file name has a space). Or make the AppImage
   executable and run it.
2. Open **GPU Share**. Within a few seconds the owner's laptop appears under **Computers**.
   If it doesn't, see "Not showing up" below.
3. Press **Ask to connect**. A 6-digit code appears. The owner sees a popup with a code too:
   **check together that the codes match**, then the owner clicks **Allow**.
4. The chat opens. Ask something. The first question takes a few seconds longer while the model
   loads onto the owner's card. If another friend's question is being answered, you'll see your
   place in line.

## Ubuntu to Ubuntu (an Ubuntu computer as the host)

Any Ubuntu computer can share too, with or without a graphics card. Handy for trying the whole
thing between two Ubuntu PCs before the Windows laptop is involved.

1. Install the package on both computers as above.
2. On the computer that will share, press **Share this computer** at the top (or **Share my
   graphics card** if it has an NVIDIA card), then **Set up sharing**, and switch sharing on.
   The first time, the app downloads the AI engine (about 30 MB) and checks it. Computers that
   never switch sharing on never download it.
3. Open **Models** and **Add** a model:
   * **Without a graphics card**, answers come from the processor, at a few words a second. Take
     **Qwen3 0.6B** (640 MB, quickest) or **Qwen3 1.7B** (1.8 GB, a bit better).
   * **With an NVIDIA card**, Qwen3 4B or 8B as on Windows, if they fit the card. The NVIDIA
     driver must be installed (Ubuntu's "Additional Drivers" does this), since the engine uses the
     card through the driver's Vulkan support.
4. If Ubuntu's firewall is on, the Share screen shows the two commands to run.
5. The other computer finds it under **Computers** and pairs exactly as with the Windows laptop.

Differences from the Windows host:

* **"Pause while I'm using the computer"** works on Ubuntu's standard desktop (GNOME). On other
  desktops (KDE, Xfce, …) the app can't tell when you're at the keyboard; the Limits screen says
  so, and the rule has no effect.
* Without a graphics card there's no temperature limit or "another app is using the GPU" rule,
  and the memory limit is the computer's RAM.
* While sharing is on, closing the window keeps the app running in the tray (top right). Use the
  tray menu's **Quit** to stop it.

## What to check during the pilot

These can only be checked on the real machines. Tick them in the roadmap's manual checklist as
you go, with a note of what you saw.

| Check | How |
|---|---|
| Pauses when the owner uses the laptop, resumes after 5 minutes away | Owner types; a friend's screen should say "They're using their computer". |
| Pauses when Blender (or another listed program) opens | Owner opens it; friends see the same general message, never the program's name. |
| Pauses when a game or other program uses the GPU | Start a game that isn't on the list. A browser playing video should **not** pause it. |
| Temperature limit | Long answers on a warm day; the Share screen shows the card's temperature. |
| Battery | Unplug the laptop's charger: sharing pauses. |
| Closing GPU Share stops the engine | After quitting from the tray, Task Manager shows no `llama-server.exe`. |
| The laptop doesn't sleep mid-answer, and does sleep later | Leave it idle after a chat. |
| Discovery on your router | Friends see the laptop appear and disappear as sharing goes on and off. |
| Speed and memory | Note load time, words per second and GPU memory for each model (roadmap `P5-T11`). |

## Not showing up?

* **Same Wi-Fi?** A guest network or a mesh router's "client isolation" keeps devices apart. The
  friend's Computers screen explains this when it detects it.
* **Network set to Public on the owner's laptop**: see step 6 above.
* **Add by IP address**: on the owner's laptop, run `ipconfig` and note the IPv4 address (like
  192.168.1.23). On the friend's computer, press **Add by IP address** and type it.
* **Ubuntu firewall**: if `ufw` is on, the app says so and shows the commands to run (on a
  friend's computer and on an Ubuntu computer that shares).
* Several network adapters (VPN, virtual machines)? Start the app with
  `GPU_SHARE_IFACE=<adapter name>` to choose the one that's on the home network.

## Privacy, in one paragraph

Nothing a friend asks or gets back is stored on the owner's laptop; the app keeps only counts
(how many questions, how many tokens, how long the card worked) per friend. Each friend's chat
history stays on their own computer. Friends never see each other's questions, and when sharing
pauses they're told "They're using their computer", not which program is open. The app can't
reach the owner's files: see [security-review.md](security-review.md).
