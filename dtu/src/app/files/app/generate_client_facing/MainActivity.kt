package com.carvesystems.example

import android.os.Bundle
import android.view.View
import androidx.appcompat.app.AppCompatActivity
import com.carvesystems.example.databinding.ActivityMainBinding

class MainActivity : AppCompatActivity() {

    private lateinit var binding: ActivityMainBinding


    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        val view = binding.root
        setContentView(view)
        binding.textView.text = INITIAL_TEXT
        binding.gotoFindingsBtn.setOnClickListener(View.OnClickListener {
            startActivity(Intent(this, FindingsListActivity::class.java))
        })
    }

    companion object {
        private val INITIAL_TEXT = """
            |Android Device Testing Application
            |
            |OEM: ${MetaData.OEM}
            |Device model: ${MetaData.MODEL}
            |Build date: ${MetaData.BUILD_DATE}
            |Build ID: ${MetaData.BUILD_ID}
            |
            |This application is intended to be a proof of concept for
            |vulnerabilities discovered by Carve Systems during device
            |testing. Failures in this application DO NOT prove that 
            |any underlying vulnerability is fixed, but may be used to
            |guide the mitigation process.
        """.trimMargin()
    }
}
