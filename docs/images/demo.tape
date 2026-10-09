# Records docs/images/demo.gif against the real API with synthetic data.
# From the repo root: make build && MLBIN_DIR=$PWD/bin DEMO_DIR=$PWD/docs/demo \
#   MIXLAYER_HOME=$(mktemp -d) vhs docs/images/demo.tape
# Needs MIXLAYER_API_KEY in the environment. Answers differ run to run.
Output docs/images/demo.gif
Set Shell "bash"
Set FontFamily "Menlo"
Set FontSize 15
Set Width 1280
Set Height 800
Set Padding 28
Set Margin 0
Set WindowBar Colorful
Set BorderRadius 8
Set TypingSpeed 28ms
Set Theme { "name": "mixlayer", "background": "#0E0E11", "foreground": "#EDE7DB", "cursor": "#FF9A4D", "selection": "#3A3A40", "black": "#0E0E11", "brightBlack": "#8b867c", "red": "#FF6B5A", "brightRed": "#FF6B5A", "green": "#7fd69a", "brightGreen": "#7fd69a", "yellow": "#FFD98A", "brightYellow": "#FFD98A", "blue": "#9ab8ff", "brightBlue": "#9ab8ff", "magenta": "#FF2E7E", "brightMagenta": "#FF2E7E", "cyan": "#8fd3d3", "brightCyan": "#8fd3d3", "white": "#EDE7DB", "brightWhite": "#ffffff" }

Hide
Type "export PS1='\[\e[38;2;255;154;77m\]$\[\e[0m\] ' PATH=$MLBIN_DIR:$PATH; cd $DEMO_DIR; clear"
Enter
Type "# Ask the top model which customers will churn. Names and companies never leave this machine."
Enter
Type "mixlayer shield ask 'Which customers will churn, and what should we do this week? One plain-text line each.' \"
Enter
Type "  --data customers.csv --mask-columns customer,company"
Enter
Wait+Screen@120s /masked \d+ before sending/
Show
Sleep 7s

Type "clear"
Enter
Type "# What the model actually saw"
Sleep 400ms
Enter
Type "mixlayer shield redact customers.csv --mask-columns customer,company | cut -d, -f1,2,4,11 | head -5"
Sleep 300ms
Enter
Sleep 3s
Type "# And the real file it came from"
Sleep 300ms
Enter
Type "cut -d, -f1,2,4,11 customers.csv | column -s, -t | head -5"
Sleep 300ms
Enter
Sleep 5s
