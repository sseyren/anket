window.onload = anket_main;

var anket = {
    canvas: {},
    socket: null,
};

function anket_getPollID() {
    var urlArr = window.location.pathname.split("/").slice(-2);
    if (urlArr.length < 2 || urlArr[0] != "p" || urlArr[1].length == 0) {
        throw new Error("Couldn't determine poll ID from this URL.");
    }
    return urlArr[1];
}

function anket_initCanvas() {
    anket.canvas.self.innerHTML = anket_HTML.canvas_init;
    anket.canvas.title = anket.canvas.self.querySelector("#anket-pollTitle");

    anket.canvas.top_items = anket.canvas.self.querySelector("#anket-itemsTop");
    anket.canvas.latest_items = anket.canvas.self.querySelector("#anket-itemsLatest");
    anket.canvas.user_items = anket.canvas.self.querySelector("#anket-itemsUser");
}

function anket_makeItem(details) {
    var id = details.id;

    var item = document.createElement("div");
    item.innerHTML = anket_HTML.item;
    item.setAttribute("anket-itemID", id);
    item.setAttribute("anket-itemUserVote", details.user_vote);

    var score = item.querySelector(".option-score");
    var content = item.querySelector(".option-content");
    var buttonUp = item.querySelector(".option-vote-up");
    var buttonDown = item.querySelector(".option-vote-down");

    score.innerText = details.score;
    content.innerText = details.text;
    buttonUp.innerText = (details.user_vote == 1) ? "⬆" : "⇧";
    buttonDown.innerText = (details.user_vote == -1) ? "⬇" : "⇩";

    buttonUp.onclick = function () {
        var current = item.getAttribute("anket-itemUserVote");
        var value = (current == "1") ? 0 : 1;
        anket_sendVoteItemMsg(id, value);
    }
    buttonDown.onclick = function () {
        var current = item.getAttribute("anket-itemUserVote");
        var value = (current == "-1") ? 0 : -1;
        anket_sendVoteItemMsg(id, value);
    }

    return item;
}

function anket_renderItems(items, target) {
    target.innerHTML = "";
    items.forEach(function (details) {
        target.appendChild(anket_makeItem(details));
    });
}

function anket_sendVoteItemMsg(itemID, voteValue) {
    anket.socket.send(JSON.stringify({
        type: "VoteItem",
        content: { item_id: itemID, vote: voteValue },
    }));
}

function anket_main() {
    anket.canvas.self = document.getElementById("poll-canvas");
    anket.canvas.self.innerHTML = "";
    var pollID = anket_getPollID();

    anket.canvas.self.innerHTML = "<p>connecting to poll...</p>";
    anket.socket = new WebSocket(`/p/${pollID}/ws`);

    anket.socket.addEventListener("close", function (event) {
        alert("websocket connection closed");
    });

    anket.socket.addEventListener("error", function (event) {
        alert("websocket connection lost");
    });

    anket.socket.addEventListener("open", function (event) {
        anket_initCanvas();
    });

    anket.socket.addEventListener("message", function (event) {
        var data = JSON.parse(event.data);
        switch (data.type) {
            case "ActionResponse":
                alert(data.content);

            case "PollStateUpdate":
                anket.canvas.title.innerText = data.content.poll_title;

                anket_renderItems(data.content.top_items, anket.canvas.top_items);
                anket_renderItems(data.content.latest_items, anket.canvas.latest_items);
                anket_renderItems(data.content.user_items, anket.canvas.user_items);
                break;
        }
    });
}

var anket_HTML = {
    "canvas_init": `
<div class="pure-g">
  <div class="pure-u-1">
    <h1 id="anket-pollTitle"></h1>
    <form class="pure-form">
      <fieldset>
        <legend>Create an option for this poll</legend>
        <input class="pure-input-3-4" type="text" placeholder="Option text" />
        <button type="submit" class="pure-button pure-input-1-4 pure-button-primary">
          Create
        </button>
      </fieldset>
    </form>
  </div>
</div>
<div class="pure-g">
  <div class="pure-u-1-3">
    <h2 class="text-center">Top Voted Items</h2>
    <div id="anket-itemsTop"></div>
  </div>
  <div class="pure-u-1-3">
    <h2 class="text-center">Recently Added Items</h2>
    <div id="anket-itemsLatest"></div>
  </div>
  <div class="pure-u-1-3">
    <h2 class="text-center">My Items</h2>
    <div id="anket-itemsUser"></div>
  </div>
</div>
`,

    "item": `
<div class="option-card">
  <div class="option-vote">
    <button class="pure-button option-vote-button option-vote-up"></button>
    <div class="option-score"></div>
    <button class="pure-button option-vote-button option-vote-down"></button>
  </div>
  <div class="option-content"></div>
</div>
`,
};
