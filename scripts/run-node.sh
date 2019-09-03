CMD="./target/release/peer_discovery -c ./config"

tmux split-window -h
tmux split-window -v
tmux select-pane -t 0
tmux split-window -v

tmux select-pane -t 0
tmux send-keys "${CMD}/master_node.yaml" C-m
tmux select-pane -t 1
tmux send-keys "${CMD}/node1.yaml" C-m
tmux select-pane -t 2
tmux send-keys "${CMD}/node2.yaml" C-m
tmux select-pane -t 3